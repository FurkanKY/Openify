//! Kulaklık profilleri: OPRA veritabanı (https://github.com/opra-project/OPRA,
//! Roon Labs; veri CC BY-SA 4.0, kod MIT).
//!
//! Veri uygulamaya GÖMÜLMEZ ve depoda durmaz: kullanıcı "Kulaklık profili"
//! aramasını ilk açtığında OPRA'nın yayınladığı tek dosya indirilir ve
//! önbelleğe yazılır (`<önbellek>/eq/`); sonraki açılışlarda önbellek hemen
//! okunur, ETag ile arkada güncellenir (bkz. `fetch`). Karar: OPRA
//! çalışırken indirilir, pakete gömülmez.
//!
//! # Dağıtım ve biçim (11 Kasım 2026'da doğrulandı)
//!
//! `https://opra.roonlabs.net/database_v1.jsonl` (Roon'un Cloudflare
//! yansısı; OPRA yönergesi açık kaynak ve kişisel kullanım için bunu söyler,
//! en çok 5 dk bayat). Tek JSONL dosyası, satır başına bir nesne:
//!
//! ```text
//! {"type":"vendor","id":"sennheiser","data":{"name":"Sennheiser"}}
//! {"type":"product","id":"sennheiser::hd650","data":{"name":"HD 650","vendor_id":"sennheiser","subtype":"over_the_ear",...}}
//! {"type":"eq","id":"...","data":{"author":"oratory1990","details":"Harman target","link":"...",
//!   "type":"parametric_eq","product_id":"sennheiser::hd650",
//!   "parameters":{"gain_db":-6.3,"bands":[{"type":"low_shelf","frequency":105,"gain_db":2.5,"q":0.7},...]}}}
//! ```
//!
//! Satır sırası belirsiz (bir `eq` satırı kendi `product` satırından önce
//! gelebilir). Ölçülen: ~19,5 bin satır; 714 üretici, 6230 ürün, 12 594 EQ;
//! 12,6 MB açık, gzip ile 1,2 MB (brotli de sunuluyor; `reqwest` gzip ister);
//! zayıf ETag (`W/"…"`) ve `Cache-Control: max-age=300` döner. EQ başına
//! ortalama 9,8 (en çok 20) bant: tepe (`peak_dip`), alçak / yüksek raf;
//! alçak geçiren nadir (5 bant); yazar çoğunlukla "AutoEQ", sonra
//! "oratory1990", "Rtings/AutoEQ".

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;

use super::equalizer::{PeqBand, PeqKind};
use super::fold_name;

/// OPRA veritabanı dosyası.
pub const URL: &str = "https://opra.roonlabs.net/database_v1.jsonl";
/// Kaynak ve lisans sayfası (arayüzdeki atıf bağlantısı).
pub const HOME_URL: &str = "https://github.com/opra-project/OPRA";
/// İndirilen dosya bundan büyükse reddedilir (bozuk ya da beklenmedik yanıt).
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const FILE_NAME: &str = "opra_database_v1.jsonl";
const META_NAME: &str = "opra_meta.json";

/// Bir ürünün (kulaklık) bir EQ profili.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// OPRA `eq` kimliği.
    pub id: String,
    /// Yaratıcı (ölçüm / EQ'yu yapan): atıfta gösterilir.
    pub author: String,
    /// "Harman Target • Standard", "Measured by …" gibi ek bilgi (boş olabilir).
    pub details: String,
    /// Kaynak bağlantısı (boş olabilir).
    pub link: String,
    pub bands: Vec<PeqBand>,
}

/// Bir kulaklık modeli ve profilleri.
#[derive(Clone, Debug)]
pub struct Product {
    pub vendor: String,
    pub name: String,
    pub profiles: Vec<Profile>,
    /// Aramada: katlanmış "üretici model" ve boşluksuz hâli.
    key: String,
    key_compact: String,
    /// Katlanmış model adı (öne eşleşme için).
    name_key: String,
}

impl Product {
    /// "Sennheiser HD 650".
    pub fn title(&self) -> String {
        if self
            .name
            .to_lowercase()
            .starts_with(&self.vendor.to_lowercase())
        {
            self.name.clone()
        } else {
            format!("{} {}", self.vendor, self.name)
        }
    }
}

/// Profil arama sonucu satırı: ürün ve profil sıraları.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub product: usize,
    pub profile: usize,
}

/// Çözümlenmiş veritabanı.
#[derive(Debug, Default)]
pub struct Catalog {
    pub products: Vec<Product>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    data: Data,
}

/// `vendor`, `product` ve `eq` verisinin ortak, hepsi isteğe bağlı gövdesi
/// (bilinmeyen alanlar yok sayılır).
#[derive(Deserialize, Default)]
#[serde(default)]
struct Data {
    name: String,
    vendor_id: String,
    product_id: String,
    author: String,
    details: String,
    link: String,
    #[serde(rename = "type")]
    eq_type: String,
    parameters: Parameters,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Parameters {
    bands: Vec<RawBand>,
}

#[derive(Deserialize)]
struct RawBand {
    #[serde(rename = "type")]
    kind: String,
    frequency: f32,
    #[serde(default)]
    gain_db: f32,
    #[serde(default)]
    q: Option<f32>,
    #[serde(default)]
    slope: Option<f32>,
}

impl RawBand {
    fn into_band(self) -> Option<PeqBand> {
        let kind = PeqKind::parse(&self.kind)?;
        if !(self.frequency.is_finite() && self.frequency > 0.0 && self.gain_db.is_finite()) {
            return None;
        }
        Some(PeqBand {
            kind,
            freq: self.frequency,
            gain_db: self.gain_db,
            q: self.q.filter(|q| q.is_finite() && *q > 0.0).unwrap_or(0.7),
            slope: self
                .slope
                .filter(|s| s.is_finite() && *s > 0.0)
                .unwrap_or(12.0),
        })
    }
}

/// Yazar sırası: önce oratory1990, sonra öbürleri (ölçüm kalitesi sırası
/// bizim işimiz değil; yalnız tutarlı ve tahmin edilebilir bir sıra).
fn author_rank(author: &str) -> u8 {
    match author {
        "oratory1990" => 0,
        "Rtings/AutoEQ" => 2,
        "AutoEQ" => 3,
        _ => 1,
    }
}

impl Catalog {
    /// JSONL metnini çözer. Bozuk satırlar, parametrik olmayan EQ'lar ve
    /// bandı olmayan profiller atlanır; hiç profil çıkmazsa hata.
    pub fn parse(text: &str) -> Result<Catalog, String> {
        use std::collections::HashMap;
        let mut vendors: HashMap<String, String> = HashMap::new();
        let mut products: HashMap<String, (String, String)> = HashMap::new();
        let mut eqs: Vec<(String, Profile)> = Vec::new();
        for raw in text.lines() {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let Ok(line) = serde_json::from_str::<Line>(raw) else {
                continue;
            };
            let d = line.data;
            match line.kind.as_str() {
                "vendor" if !d.name.is_empty() => {
                    vendors.insert(line.id, d.name);
                }
                "product" if !d.name.is_empty() => {
                    products.insert(line.id, (d.vendor_id, d.name));
                }
                "eq" if (d.eq_type.is_empty() || d.eq_type == "parametric_eq")
                    && !d.product_id.is_empty() =>
                {
                    let bands: Vec<PeqBand> = d
                        .parameters
                        .bands
                        .into_iter()
                        .filter_map(RawBand::into_band)
                        .collect();
                    if bands.is_empty() {
                        continue;
                    }
                    eqs.push((
                        d.product_id,
                        Profile {
                            id: line.id,
                            author: d.author,
                            details: d.details,
                            link: d.link,
                            bands,
                        },
                    ));
                }
                _ => {}
            }
        }
        let mut grouped: HashMap<String, Vec<Profile>> = HashMap::new();
        for (product_id, profile) in eqs {
            grouped.entry(product_id).or_default().push(profile);
        }
        let mut out: Vec<Product> = grouped
            .into_iter()
            .filter_map(|(product_id, mut profiles)| {
                let (vendor_id, name) = products.get(&product_id)?;
                let vendor = vendors
                    .get(vendor_id)
                    .cloned()
                    .unwrap_or_else(|| vendor_id.clone());
                // Aynı yaratıcı + aynı ayrıntı (ör. aynı ölçümün 5 ve 10 bantlı sürümü):
                // ekranda ayırt edilemezler; bantı en çok olan kalır (kendi 10 bandımıza
                // en iyi uydurulur).
                profiles.sort_by(|a, b| {
                    (
                        author_rank(&a.author),
                        &a.author,
                        &a.details,
                        std::cmp::Reverse(a.bands.len()),
                    )
                        .cmp(&(
                            author_rank(&b.author),
                            &b.author,
                            &b.details,
                            std::cmp::Reverse(b.bands.len()),
                        ))
                });
                profiles.dedup_by(|b, a| a.author == b.author && a.details == b.details);
                let title = format!("{vendor} {name}");
                let key = fold_name(&title);
                Some(Product {
                    key_compact: key.replace(' ', ""),
                    name_key: fold_name(name),
                    key,
                    vendor,
                    name: name.clone(),
                    profiles,
                })
            })
            .collect();
        if out.is_empty() {
            return Err("no parametric profiles in the database".into());
        }
        out.sort_by(|a, b| (&a.vendor, &a.name).cmp(&(&b.vendor, &b.name)));
        Ok(Catalog { products: out })
    }

    /// Toplam profil sayısı (testler).
    #[cfg(test)]
    pub fn profile_count(&self) -> usize {
        self.products.iter().map(|p| p.profiles.len()).sum()
    }

    /// Bellekteki yaklaşık boyut (bayt): dizgi ve vektör kapasiteleri, ayırıcı payı hariç (ölçüm testi).
    #[cfg(test)]
    pub fn approx_bytes(&self) -> usize {
        use std::mem::size_of;
        let profile = |q: &Profile| {
            q.id.capacity()
                + q.author.capacity()
                + q.details.capacity()
                + q.link.capacity()
                + q.bands.capacity() * size_of::<PeqBand>()
        };
        self.products.capacity() * size_of::<Product>()
            + self
                .products
                .iter()
                .map(|p| {
                    p.vendor.capacity()
                        + p.name.capacity()
                        + p.key.capacity()
                        + p.key_compact.capacity()
                        + p.name_key.capacity()
                        + p.profiles.capacity() * size_of::<Profile>()
                        + p.profiles.iter().map(profile).sum::<usize>()
                })
                .sum::<usize>()
    }

    /// Marka / model araması: sorgunun her sözcüğü "üretici model" içinde
    /// geçmeli (boşluksuz hâli de sayılır: "hd650" = "HD 650"). Önce model
    /// adı sorguyla başlayanlar, sonra kısa adlılar. En çok `max_rows` satır
    /// (ürün başına profilleri sırayla).
    pub fn search(&self, query: &str, max_rows: usize) -> Vec<Hit> {
        let folded = fold_name(query);
        let tokens: Vec<&str> = folded.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.is_empty() {
            return Vec::new();
        }
        let joined = folded.replace(' ', "");
        let mut scored: Vec<(i32, usize)> = Vec::new();
        for (i, p) in self.products.iter().enumerate() {
            let all = tokens
                .iter()
                .all(|t| p.key.contains(t) || p.key_compact.contains(t));
            if !all && !p.key_compact.contains(&joined) {
                continue;
            }
            // Düşük puan önce: kısa adlılar, sorgu sözcüğüyle BAŞLAYAN sözcükler
            // ("hd 6" için "HD 600", "HD 560S"ten önce) ve ilk sözcüğü öne gelenler.
            let mut score = p.key.len() as i32;
            let word_starts = tokens
                .iter()
                .filter(|t| p.key.split(' ').any(|w| w.starts_with(**t)))
                .count() as i32;
            score -= 60 * word_starts;
            if p.name_key.starts_with(tokens[0]) || p.key.starts_with(tokens[0]) {
                score -= 100;
            }
            if p.key_compact == joined || p.name_key.replace(' ', "") == joined {
                score = -10_000;
            }
            scored.push((score, i));
        }
        scored.sort();
        let mut rows = Vec::new();
        'outer: for (_, product) in scored {
            for profile in 0..self.products[product].profiles.len() {
                rows.push(Hit { product, profile });
                if rows.len() >= max_rows {
                    break 'outer;
                }
            }
        }
        rows
    }

    /// Kimliğine göre profil (testler).
    #[cfg(test)]
    pub fn find(&self, id: &str) -> Option<(&Product, &Profile)> {
        self.products
            .iter()
            .find_map(|p| p.profiles.iter().find(|q| q.id == id).map(|q| (p, q)))
    }
}

// ── Önbellek ve indirme ────────────────────────────────────────────────

/// Önbellek klasörü: `<önbellek>/eq/` (sınamada kum havuzu / `OPENIFY_TEST_CACHE_DIR`).
pub fn cache_dir() -> Option<PathBuf> {
    Some(super::cache::cache_dir()?.join("eq"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// Ağ yok / sunucu yanıt vermedi / hata kodu.
    Network,
    /// Dosya geldi ama anlaşılmadı.
    Format,
}

/// Önbellekteki veritabanı (yoksa ya da bozuksa `None`).
pub fn read_cached(dir: &Path) -> Option<Catalog> {
    let text = fs::read_to_string(dir.join(FILE_NAME)).ok()?;
    Catalog::parse(&text).ok()
}

/// Önbelleğe atomik yazar (geçici dosya + taşıma) ve ETag'i kaydeder.
fn write_cache(dir: &Path, text: &str, etag: Option<&str>) {
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        fs::write(&tmp, text)?;
        fs::rename(&tmp, dir.join(FILE_NAME))?;
        let meta = serde_json::json!({ "etag": etag, "url": URL });
        fs::write(dir.join(META_NAME), meta.to_string())
    };
    if let Err(e) = write() {
        log::warn!("OPRA önbelleği yazılamadı ({}): {e}", dir.display());
    }
}

fn read_etag(dir: &Path) -> Option<String> {
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join(META_NAME)).ok()?).ok()?;
    meta.get("etag")?.as_str().map(str::to_owned)
}

/// Verinin nereden geleceği.
pub enum Source {
    /// Gerçek indirme (ETag'li).
    Network,
    /// Ağa çıkılmaz (çevrimdışı sınama koşusu).
    Offline,
    /// Sınama kancası: yerel dosya "indirme" gibi önbelleğe yazılır.
    Local(String),
    /// Sınama kancası: indirme hatası.
    Broken,
}

impl Source {
    /// Ortamdan seçer. `OPENIFY_TEST_OPRA_FILE=<dosya>` yerel dosyadan besler,
    /// `!error` indirme hatasını taklit eder; ikisi de yalnız sınama koşusunda
    /// (`is_test_run`) geçerli. Sınama koşusu varsayılan olarak çevrimdışıdır.
    pub fn from_env() -> Source {
        let test = crate::crash::is_test_run();
        if test && let Ok(v) = std::env::var("OPENIFY_TEST_OPRA_FILE") {
            if v.starts_with('!') {
                return Source::Broken;
            }
            return match fs::read_to_string(&v) {
                Ok(text) => Source::Local(text),
                Err(e) => {
                    log::warn!("OPENIFY_TEST_OPRA_FILE okunamadı ({v}): {e}");
                    Source::Broken
                }
            };
        }
        if std::env::var_os("OPENIFY_TEST_OFFLINE").is_some()
            || (test && std::env::var_os("OPENIFY_TEST_ONLINE").is_none())
        {
            return Source::Offline;
        }
        Source::Network
    }
}

/// Koşulu GET: `Ok(None)` = değişmedi (304), `Ok(Some((metin, etag)))` = yeni.
async fn download(
    client: &reqwest::Client,
    etag: Option<&str>,
) -> Result<Option<(String, Option<String>)>, FetchError> {
    let mut request = client.get(URL);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().await.map_err(|e| {
        log::warn!("OPRA indirilemedi: {e}");
        FetchError::Network
    })?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !response.status().is_success() {
        log::warn!("OPRA indirilemedi: HTTP {}", response.status());
        return Err(FetchError::Network);
    }
    if response.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err(FetchError::Format);
    }
    let new_etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = response.bytes().await.map_err(|e| {
        log::warn!("OPRA gövdesi okunamadı: {e}");
        FetchError::Network
    })?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(FetchError::Format);
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| FetchError::Format)?;
    Ok(Some((text, new_etag)))
}

fn network_fetch(
    have_cache: bool,
    dir: &Path,
) -> Result<Option<(String, Option<String>)>, FetchError> {
    // Kısa ömürlü tek iş parçacıklı çalışma ortamı (çağıran bir iş parçacığı).
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| FetchError::Network)?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent("Openify")
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|_| FetchError::Network)?;
        // ETag yalnız önbellek varsa işe yarar (yoksa 304'te elimizde veri olmaz).
        let etag = if have_cache { read_etag(dir) } else { None };
        download(&client, etag.as_deref()).await
    })
}

/// Arayüzün istediği iş: önbelleği hemen ver (`on_ready`), sonra güncelle
/// (yeni veri gelirse yine `on_ready`). Önbellek yoksa indirme başarısızlığı
/// `Err`; önbellek varsa güncelleme hatası sessizdir (günlük). Bloklar:
/// çağıran bir iş parçacığıdır.
pub fn load_blocking(
    dir: &Path,
    source: Source,
    mut on_ready: impl FnMut(Catalog),
) -> Result<(), FetchError> {
    let cached = read_cached(dir);
    let have_cache = cached.is_some();
    if let Some(catalog) = cached {
        on_ready(catalog);
    }
    let fetched = match source {
        Source::Offline | Source::Broken => Err(FetchError::Network),
        Source::Local(text) => Ok(Some((text, Some("test".to_string())))),
        Source::Network => network_fetch(have_cache, dir),
    };
    match fetched {
        Ok(None) => Ok(()),
        Ok(Some((text, etag))) => match Catalog::parse(&text) {
            Ok(catalog) => {
                write_cache(dir, &text, etag.as_deref());
                on_ready(catalog);
                Ok(())
            }
            Err(e) => {
                log::warn!("OPRA dosyası anlaşılmadı: {e}");
                if have_cache {
                    Ok(())
                } else {
                    Err(FetchError::Format)
                }
            }
        },
        Err(e) if have_cache => {
            log::info!("OPRA güncellenemedi, önbellek kullanılıyor: {e:?}");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Küçük, sentetik OPRA benzeri dosya (gerçek veri parçası değil): satır
    /// sırası bilerek karışık (eq, ürün satırından önce), bozuk satır, bandı
    /// olmayan eq, parametrik olmayan eq, bilinmeyen alanlar.
    pub(super) const FIXTURE: &str = r#"
{"type":"eq","id":"acme:alpha::lab_one","data":{"author":"Lab One","details":"Test target • Standard","link":"https://example.test/a","type":"parametric_eq","parameters":{"gain_db":-3,"bands":[{"type":"low_shelf","frequency":105,"gain_db":3.5,"q":0.7},{"type":"peak_dip","frequency":3000,"gain_db":-2,"q":1.5},{"type":"high_shelf","frequency":10000,"gain_db":-1,"q":0.7}]},"product_id":"acme::alpha"}}
{"type":"vendor","id":"acme","data":{"name":"Acme Audio","extra":"x"}}
{"type":"product","id":"acme::alpha","data":{"name":"Alpha 650","type":"headphones","subtype":"over_the_ear","vendor_id":"acme","line_art_svg":"assets/x.svg"}}
{"type":"product","id":"acme::beta","data":{"name":"Beta","type":"headphones","subtype":"in_ear","vendor_id":"acme"}}
this line is not json
{"type":"eq","id":"acme:alpha::other","data":{"author":"AutoEQ","details":"Measured by Someone","type":"parametric_eq","parameters":{"gain_db":-1,"bands":[{"type":"peak_dip","frequency":1000,"gain_db":2,"q":1}]},"product_id":"acme::alpha"}}
{"type":"eq","id":"acme:alpha::first","data":{"author":"oratory1990","details":"","type":"parametric_eq","parameters":{"gain_db":0,"bands":[{"type":"low_pass","frequency":9000,"slope":12}]},"product_id":"acme::alpha"}}
{"type":"eq","id":"acme:beta::empty","data":{"author":"X","type":"parametric_eq","parameters":{"gain_db":0,"bands":[]},"product_id":"acme::beta"}}
{"type":"eq","id":"acme:beta::graphic","data":{"author":"X","type":"graphic_eq","parameters":{"gain_db":0,"bands":[{"type":"peak_dip","frequency":1000,"gain_db":2,"q":1}]},"product_id":"acme::beta"}}
{"type":"vendor","id":"zed","data":{"name":"Zed"}}
{"type":"product","id":"zed::mk2","data":{"name":"Zed Pro MK2","type":"headphones","subtype":"on_ear","vendor_id":"zed"}}
{"type":"eq","id":"zed:mk2::lab","data":{"author":"Lab Two","details":"Another","type":"parametric_eq","parameters":{"gain_db":-2,"bands":[{"type":"band_pass","frequency":500,"gain_db":0,"q":2},{"type":"mystery","frequency":1,"q":1},{"type":"peak_dip","frequency":-5,"gain_db":1,"q":1}]},"product_id":"zed::mk2"}}
{"type":"eq","id":"orphan::eq","data":{"author":"Nobody","type":"parametric_eq","parameters":{"gain_db":0,"bands":[{"type":"peak_dip","frequency":1000,"gain_db":1,"q":1}]},"product_id":"missing::product"}}
"#;

    fn catalog() -> Catalog {
        Catalog::parse(FIXTURE).expect("fixture parses")
    }

    #[test]
    fn parse_groups_profiles_by_product_in_any_line_order() {
        let c = catalog();
        // beta (bandsız / grafik eq) ve yetim eq elenir.
        assert_eq!(c.products.len(), 2);
        assert_eq!(c.profile_count(), 4);
        let alpha = c.products.iter().find(|p| p.name == "Alpha 650").unwrap();
        assert_eq!(alpha.vendor, "Acme Audio");
        assert_eq!(alpha.title(), "Acme Audio Alpha 650");
        // oratory1990 önce, sonra öbürleri, AutoEQ en sonda.
        let order: Vec<&str> = alpha.profiles.iter().map(|p| p.author.as_str()).collect();
        assert_eq!(order, ["oratory1990", "Lab One", "AutoEQ"]);
        let lab = &alpha.profiles[1];
        assert_eq!(lab.details, "Test target • Standard");
        assert_eq!(lab.link, "https://example.test/a");
        assert_eq!(lab.bands.len(), 3);
        assert_eq!(lab.bands[0].kind, PeqKind::LowShelf);
        assert_eq!(lab.bands[1].q, 1.5);
        // Alçak geçiren eğimi okunur; q yoksa varsayılan.
        let lp = &alpha.profiles[0].bands[0];
        assert_eq!((lp.kind, lp.slope), (PeqKind::LowPass, 12.0));
        // Bilinmeyen tür ve eksi frekanslı bant atlanır, band_pass kalır.
        let zed = c.products.iter().find(|p| p.name == "Zed Pro MK2").unwrap();
        assert_eq!(zed.vendor, "Zed");
        assert_eq!(
            zed.title(),
            "Zed Pro MK2",
            "ad üretici adıyla başlıyorsa tekrarlanmaz"
        );
        assert_eq!(zed.profiles[0].bands.len(), 1);
        assert_eq!(zed.profiles[0].bands[0].kind, PeqKind::BandPass);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Catalog::parse("").is_err());
        assert!(Catalog::parse("not json\n{}\n").is_err());
        assert!(Catalog::parse(r#"{"type":"vendor","id":"a","data":{"name":"A"}}"#).is_err());
    }

    #[test]
    fn search_matches_brand_and_model_loosely() {
        let c = catalog();
        let titles = |q: &str| -> Vec<String> {
            c.search(q, 20)
                .iter()
                .map(|h| c.products[h.product].title())
                .collect::<Vec<_>>()
        };
        assert!(titles("").is_empty());
        assert!(titles("   ").is_empty());
        assert_eq!(titles("acme").len(), 3, "üç profil satırı");
        assert_eq!(titles("alpha 650")[0], "Acme Audio Alpha 650");
        assert_eq!(
            titles("alpha650")[0],
            "Acme Audio Alpha 650",
            "boşluksuz yazım"
        );
        assert_eq!(
            titles("ALPHA")[0],
            "Acme Audio Alpha 650",
            "büyük / küçük harf"
        );
        assert_eq!(titles("zed mk2"), ["Zed Pro MK2"]);
        assert_eq!(titles("acme audio zed").len(), 0);
        assert!(titles("nonexistent").is_empty());
        // Satır sınırı profil satırlarını da keser.
        assert_eq!(c.search("acme", 2).len(), 2);
        // Ürünün profilleri bitişik ve sıralı.
        let hits = c.search("alpha", 10);
        assert_eq!(
            hits.iter().map(|h| h.profile).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    /// Aynı yaratıcı + ayrıntıyla ayırt edilemeyen profillerden bandı çok olan
    /// kalır; "hd 6" sorgusunda 6 ile başlayan modeller 560'tan önce gelir.
    #[test]
    fn identical_labels_keep_the_richest_profile_and_word_starts_rank_first() {
        let eq = |id: &str, product: &str, bands: usize| {
            let band = r#"{"type":"peak_dip","frequency":1000,"gain_db":1,"q":1}"#;
            let list = vec![band; bands].join(",");
            format!(
                r#"{{"type":"eq","id":"{id}","data":{{"author":"Rtings/AutoEQ","details":"Same label","type":"parametric_eq","parameters":{{"gain_db":0,"bands":[{list}]}},"product_id":"{product}"}}}}"#
            )
        };
        let product = |id: &str, name: &str| {
            format!(
                r#"{{"type":"product","id":"{id}","data":{{"name":"{name}","vendor_id":"v"}}}}"#
            )
        };
        let text = [
            r#"{"type":"vendor","id":"v","data":{"name":"Vee"}}"#.to_string(),
            product("v::a", "HD 560S"),
            product("v::b", "HD 600"),
            eq("a5", "v::a", 5),
            eq("a10", "v::a", 10),
            eq("b5", "v::b", 5),
        ]
        .join("\n");
        let c = Catalog::parse(&text).unwrap();
        assert_eq!(c.profile_count(), 2, "5 bantlı ikiz elendi");
        assert_eq!(c.find("a10").map(|(_, q)| q.bands.len()), Some(10));
        assert!(c.find("a5").is_none());
        let first = c.search("hd 6", 5);
        assert_eq!(
            c.products[first[0].product].name, "HD 600",
            "6 ile başlayan sözcük önce"
        );
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn find_by_id() {
        let c = catalog();
        let (p, q) = c.find("acme:alpha::lab_one").unwrap();
        assert_eq!(
            (p.name.as_str(), q.author.as_str()),
            ("Alpha 650", "Lab One")
        );
        assert!(c.find("nope").is_none());
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openify-opra-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn cache_roundtrip_and_etag() {
        let dir = temp_dir("cache");
        assert!(read_cached(&dir).is_none());
        assert!(read_etag(&dir).is_none());
        write_cache(&dir, FIXTURE, Some("W/\"abc\""));
        assert_eq!(read_cached(&dir).unwrap().profile_count(), 4);
        assert_eq!(read_etag(&dir).as_deref(), Some("W/\"abc\""));
        assert!(
            !dir.join(format!("{FILE_NAME}.tmp")).exists(),
            "geçici dosya kalmadı"
        );
        // Bozuk önbellek yok sayılır.
        fs::write(dir.join(FILE_NAME), "garbage").unwrap();
        assert!(read_cached(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// İndirme akışı (yerel kaynakla): ilk yükleme dosyayı önbelleğe yazar ve
    /// kataloğu verir; sonra ağ yokken önbellekten sessizce devam edilir; ne
    /// önbellek ne ağ varsa hata; bozuk dosya önbelleği bozmaz.
    #[test]
    fn load_flow_downloads_caches_and_falls_back() {
        let dir = temp_dir("flow");
        // Önbellek yok, ağ yok: hata, sessiz değil.
        let mut calls = 0;
        assert_eq!(
            load_blocking(&dir, Source::Broken, |_| calls += 1),
            Err(FetchError::Network)
        );
        assert_eq!(
            load_blocking(&dir, Source::Offline, |_| calls += 1),
            Err(FetchError::Network)
        );
        assert_eq!(calls, 0);
        // İlk "indirme": katalog bir kez gelir, dosya yazılır.
        let mut sizes = Vec::new();
        assert_eq!(
            load_blocking(&dir, Source::Local(FIXTURE.to_string()), |c| sizes
                .push(c.products.len())),
            Ok(())
        );
        assert_eq!(sizes, [2]);
        assert!(dir.join(FILE_NAME).exists() && dir.join(META_NAME).exists());
        assert_eq!(read_etag(&dir).as_deref(), Some("test"));
        // Ağ yok: önbellek hemen verilir, hata yok.
        let mut sizes = Vec::new();
        assert_eq!(
            load_blocking(&dir, Source::Broken, |c| sizes.push(c.products.len())),
            Ok(())
        );
        assert_eq!(sizes, [2]);
        // Önbellek + yeni veri: önce önbellek, sonra yenisi.
        let extra = r#"{"type":"eq","id":"zed:mk2::new","data":{"author":"N","type":"parametric_eq","parameters":{"gain_db":0,"bands":[{"type":"peak_dip","frequency":1000,"gain_db":1,"q":1}]},"product_id":"zed::mk2"}}"#;
        let newer = format!("{FIXTURE}\n{extra}\n");
        let mut counts = Vec::new();
        assert_eq!(
            load_blocking(&dir, Source::Local(newer), |c| counts
                .push(c.profile_count())),
            Ok(())
        );
        assert_eq!(counts, [4, 5]);
        // Bozuk "indirme" önbelleği bozmaz.
        let mut counts = Vec::new();
        assert_eq!(
            load_blocking(&dir, Source::Local("garbage".into()), |c| counts
                .push(c.profile_count())),
            Ok(())
        );
        assert_eq!(counts, [5]);
        let _ = fs::remove_dir_all(&dir);
        // Önbelleksiz bozuk dosya: biçim hatası, önbelleğe yazılmaz.
        assert_eq!(
            load_blocking(&dir, Source::Local("garbage".into()), |_| {}),
            Err(FetchError::Format)
        );
        assert!(
            !dir.join(FILE_NAME).exists(),
            "bozuk veri önbelleğe yazılmadı"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// GELİŞTİRME ölçümü (CI'da koşmaz): gerçek OPRA dosyasında çözümleme ve
    /// yaklaşım hatası. `OPENIFY_TEST_OPRA_FILE=<indirilmiş dosya> cargo test
    /// --release opra_real_fit_error -- --ignored --nocapture`.
    #[test]
    #[ignore = "gerçek OPRA dosyası ister (OPENIFY_TEST_OPRA_FILE); yalnız ölçüm"]
    fn opra_real_fit_error() {
        let path =
            std::env::var("OPENIFY_TEST_OPRA_FILE").expect("OPENIFY_TEST_OPRA_FILE verilmeli");
        let text = fs::read_to_string(path).unwrap();
        let t0 = std::time::Instant::now();
        let c = Catalog::parse(&text).unwrap();
        println!(
            "çözümleme: {} ürün, {} profil, {:?}, yaklaşık {:.1} MB bellekte",
            c.products.len(),
            c.profile_count(),
            t0.elapsed(),
            c.approx_bytes() as f64 / 1048576.0
        );
        let mut rms: Vec<f32> = Vec::new();
        let mut max: Vec<f32> = Vec::new();
        let mut ratio: Vec<f32> = Vec::new();
        let step: usize = std::env::var("OPENIFY_FIT_STEP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let t1 = std::time::Instant::now();
        let mut n = 0usize;
        for p in &c.products {
            for q in &p.profiles {
                n += 1;
                if n % step != 0 {
                    continue;
                }
                let f = super::super::equalizer::fit(&q.bands);
                rms.push(f.rms_db);
                max.push(f.max_db);
                if f.target_rms_db > 0.05 {
                    ratio.push(f.rms_db / f.target_rms_db);
                }
            }
        }
        let per = t1.elapsed().as_secs_f64() * 1000.0 / rms.len() as f64;
        let pct = |v: &mut Vec<f32>, p: f64| -> f32 {
            v.sort_by(|a, b| a.total_cmp(b));
            v[((v.len() - 1) as f64 * p) as usize]
        };
        println!("uydurma: {} profil, profil başına {:.2} ms", rms.len(), per);
        println!(
            "RMS (dB): ortalama {:.3}, ortanca {:.3}, %90 {:.3}, %99 {:.3}",
            rms.iter().sum::<f32>() / rms.len() as f32,
            pct(&mut rms.clone(), 0.5),
            pct(&mut rms.clone(), 0.9),
            pct(&mut rms.clone(), 0.99)
        );
        println!(
            "en büyük sapma (dB): ortalama {:.3}, ortanca {:.3}, %90 {:.3}, %99 {:.3}, en kötü {:.3}",
            max.iter().sum::<f32>() / max.len() as f32,
            pct(&mut max.clone(), 0.5),
            pct(&mut max.clone(), 0.9),
            pct(&mut max.clone(), 0.99),
            pct(&mut max.clone(), 1.0)
        );
        println!(
            "kalan hata / hedefin sapması: ortanca {:.3}, %90 {:.3}",
            pct(&mut ratio.clone(), 0.5),
            pct(&mut ratio.clone(), 0.9)
        );
    }
}
