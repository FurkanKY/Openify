//! Arayüz dili: İngilizce ve Türkçe.
//!
//! Neden elle yazılmış, bağımlılıksız bir tablo? Fluent / gettext türü kütüphaneler
//! çalışma anında dosya okur ve eksik anahtarı boş / anahtar adı olarak gösterir;
//! burada iki dil, tek bir makro girdisinde (`src/i18n/table.rs`) birlikte yazılır:
//! bir dilin eksik kalması DERLEME HATASI (makro iki alanı da ister), anahtar adı
//! yanlış yazılırsa da derleme hatası (anahtarlar bir `enum`). Yer tutucu sayısı iki
//! dilde aynı mı, çoğul biçimi doğru mu, boş yazı var mı gibi şeyleri aşağıdaki
//! testler yakalar. Ek bağımlılık yok, ikilide tablo `&'static str` olarak gömülü.
//!
//! Kullanım:
//!   `t!(NavHome)`            → `&'static str` (düz yazı)
//!   `tf!(AddedTo, name)`     → `String` (`{}` yer tutucuları sırayla; `{0}` `{1}` ile sıra değişir)
//!   `tp!(Songs, n)`          → `String` (çoğul: İngilizce "{} song|{} songs", sayı gruplanır)
//!   `k!(NavHome)`            → `K` değeri (sabit tablolarda anahtar saklamak için; `i18n::t(k)`)
//!
//! Dil: açılışta Windows'un görüntü dili (Türkçe ise `tr`, değilse `en`); Ayarlar'da
//! `Sistem / English / Türkçe` ile değiştirilir ve hemen uygulanır (kök görünüm
//! yenilenir, bütün görünümler onu izler). Sınama kancası: `OPENIFY_TEST_LANG=en|tr`.
//!
//! Görünen ad ve belirtme hâli `src/brand.rs`'ten gelir (`{app}` yer tutucusu yok:
//! tablodaki yazılar `concat!` ile derleme zamanında birleşir).

use std::fmt::{Display, Write as _};
#[cfg(not(test))]
use std::sync::atomic::Ordering;
use std::sync::atomic::AtomicU8;

/// Anahtar tablosunu ve `K` sıralamasını kuran makro. Her girdi İKİ dili birden
/// ister: biri eksikse derleme durur.
macro_rules! keys {
    ($( $name:ident { en: $en:expr, tr: $tr:expr $(,)? } )*) => {
        /// Arayüz yazısının anahtarı.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[allow(dead_code)]
        pub enum K { $($name),* }
        /// Bütün anahtarlar (testler için).
        #[allow(dead_code)]
        pub const ALL: &[K] = &[$(K::$name),*];
        pub(super) const EN: &[&str] = &[$($en),*];
        pub(super) const TR: &[&str] = &[$($tr),*];
        #[allow(dead_code)]
        pub(super) const NAMES: &[&str] = &[$(stringify!($name)),*];
    };
}

pub mod canon;
mod table;
pub use table::K;

/// Arayüz dili.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Tr,
}

impl Lang {
    /// BCP 47 dil kodu (`en`, `tr`): ağ isteklerinin `accept-language` başlığı gibi
    /// yerlerde, yeni bir dil eklenince tek yerden genişler.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Tr => "tr",
        }
    }
}

/// Ayardaki dil seçenekleri (`settings.json`'da `language`): sistem diline uy / İngilizce / Türkçe.
pub const PREFS: [&str; 3] = ["system", "en", "tr"];

#[cfg_attr(test, allow(dead_code))]
const UNSET: u8 = 255;
#[cfg_attr(test, allow(dead_code))]
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);

#[cfg(test)]
thread_local! {
    static TEST_LANG: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) };
}

/// Şu anki dil. İlk çağrıda sistem diline (ya da sınama kancasına) göre kurulur,
/// sonra `apply_preference` ile ayara göre değişir.
///
/// Birim testlerde dil iş parçacığına özeldir ve varsayılanı Türkçedir (mevcut
/// testler Türkçe yazıları bekler, paralel testler birbirini bozmaz); İngilizce
/// için `with_lang`.
pub fn lang() -> Lang {
    #[cfg(test)]
    {
        TEST_LANG.with(|l| l.get()).unwrap_or(Lang::Tr)
    }
    #[cfg(not(test))]
    {
        match CURRENT.load(Ordering::Relaxed) {
            0 => Lang::En,
            1 => Lang::Tr,
            _ => {
                let l = resolve("system");
                set(l);
                l
            }
        }
    }
}

#[cfg_attr(test, allow(dead_code))]
fn set(l: Lang) {
    #[cfg(test)]
    TEST_LANG.with(|t| t.set(Some(l)));
    #[cfg(not(test))]
    CURRENT.store(if l == Lang::Tr { 1 } else { 0 }, Ordering::Relaxed);
}

/// Testte yalnız bu iş parçacığında, `f` süresince dili `l` yap.
#[cfg(test)]
pub fn with_lang<R>(l: Lang, f: impl FnOnce() -> R) -> R {
    let before = TEST_LANG.with(|t| t.replace(Some(l)));
    let out = f();
    TEST_LANG.with(|t| t.set(before));
    out
}

/// Ayar değerinden dili bulur. `OPENIFY_TEST_LANG` (en / tr) her şeyin üstündedir
/// (ekran sınamaları); `system` ve bilinmeyen değerler Windows'un dilini izler.
pub fn resolve(pref: &str) -> Lang {
    if let Some(l) = test_override(std::env::var("OPENIFY_TEST_LANG").ok().as_deref()) {
        return l;
    }
    match pref {
        "en" => Lang::En,
        "tr" => Lang::Tr,
        _ => system_lang(),
    }
}

fn test_override(value: Option<&str>) -> Option<Lang> {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("en") => Some(Lang::En),
        Some("tr") => Some(Lang::Tr),
        _ => None,
    }
}

/// Ayarı uygular (açılışta ve Ayarlar'da dil değişince). Döndürdüğü değer dilin
/// değişip değişmediği: değiştiyse sistem parçaları (tepsi, medya, görev çubuğu)
/// yazılarını yenilemeli.
pub fn apply_preference(pref: &str) -> bool {
    let before = lang();
    let after = resolve(pref);
    set(after);
    before != after
}

/// Windows'un görüntü dili: Türkçe ise `Tr`, değilse `En`.
pub fn system_lang() -> Lang {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // SAFETY: bağımsız değişkensiz, yalnız sayı döndüren Win32 çağrısı.
        lang_from_langid(unsafe { GetUserDefaultUILanguage() })
    }
    #[cfg(not(windows))]
    {
        Lang::En
    }
}

/// LANGID'in birincil dil kısmı (alt 10 bit) Türkçe (0x1F) ise `Tr`.
pub fn lang_from_langid(id: u16) -> Lang {
    if id & 0x3ff == 0x1f { Lang::Tr } else { Lang::En }
}

/// Düz yazı.
pub fn t(k: K) -> &'static str {
    match lang() {
        Lang::En => table::EN[k as usize],
        Lang::Tr => table::TR[k as usize],
    }
}

/// Yer tutuculu yazı: `{}` sırayla, `{0}` / `{1}` sıra belirterek.
pub fn tf(k: K, args: &[&dyn Display]) -> String {
    fill(t(k), args)
}

/// Çoğul yazı: `tek|çoğul` biçiminde (İngilizce "{} song|{} songs"); sayı 1 ise
/// birincisi, değilse ikincisi. `|` yoksa (Türkçe: "{} şarkı") tek biçim. İlk `{}`
/// (`{0}`) gruplanmış sayıdır, sonrakiler `extra`'dır ("Couldn't add {} song to {}").
pub fn tp(k: K, n: u64, extra: &[&dyn Display]) -> String {
    let s = pick_plural(t(k), n);
    let count = num(n);
    let mut args: Vec<&dyn Display> = Vec::with_capacity(1 + extra.len());
    args.push(&count);
    args.extend_from_slice(extra);
    fill(s, &args)
}

fn pick_plural(s: &'static str, n: u64) -> &'static str {
    match s.split_once('|') {
        Some((one, many)) => if n == 1 { one } else { many },
        None => s,
    }
}

fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0usize;
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        let index = if name.is_empty() {
            let i = next;
            next += 1;
            Some(i)
        } else {
            name.parse::<usize>().ok()
        };
        match index.and_then(|i| args.get(i)) {
            Some(arg) => {
                let _ = write!(out, "{arg}");
            }
            // Bilinmeyen yer tutucu: olduğu gibi bırak (testler yakalar).
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Binlik ayraçlı sayı: Türkçe "1.234", İngilizce "1,234".
pub fn num(n: u64) -> String {
    group(n, if lang() == Lang::Tr { '.' } else { ',' })
}

fn group(n: u64, sep: char) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// Hazır biçimlenmiş sayıdaki ondalık noktayı dile uydurur ("1.2 MB" → Türkçede "1,2 MB").
pub fn localize_decimal(s: &str) -> String {
    if lang() == Lang::Tr { s.replace('.', ",") } else { s.to_string() }
}

/// Küçük harfe çevirme; Türkçede İ → i, I → ı (Rust'ın varsayılanı "i̇" üretir).
pub fn lower(s: &str) -> String {
    if lang() != Lang::Tr {
        return s.to_lowercase();
    }
    s.chars()
        .flat_map(|c| match c {
            'İ' => vec!['i'],
            'I' => vec!['ı'],
            c => c.to_lowercase().collect(),
        })
        .collect()
}

/// Büyük harfe çevirme; Türkçede i → İ, ı → I.
pub fn upper(s: &str) -> String {
    if lang() != Lang::Tr {
        return s.to_uppercase();
    }
    s.chars()
        .flat_map(|c| match c {
            'i' => vec!['İ'],
            'ı' => vec!['I'],
            c => c.to_uppercase().collect(),
        })
        .collect()
}

/// Düz yazı (`&'static str`): `t!(NavHome)`.
#[macro_export]
macro_rules! t {
    ($key:ident) => {
        $crate::i18n::t($crate::i18n::K::$key)
    };
}

/// Yer tutuculu yazı (`String`): `tf!(AddedTo, name)`.
#[macro_export]
macro_rules! tf {
    ($key:ident $(, $arg:expr)* $(,)?) => {
        $crate::i18n::tf($crate::i18n::K::$key, &[$(&$arg as &dyn ::std::fmt::Display),*])
    };
}

/// Çoğul yazı (`String`): `tp!(Songs, n)` ya da fazladan bağımsız değişkenlerle
/// `tp!(FailedAddToList, n, list_name)`.
#[macro_export]
macro_rules! tp {
    ($key:ident, $n:expr $(, $arg:expr)* $(,)?) => {
        $crate::i18n::tp($crate::i18n::K::$key, ($n) as u64, &[$(&$arg as &dyn ::std::fmt::Display),*])
    };
}

/// Anahtarın kendisi (`K`): sabit tablolarda yazı yerine anahtar tutmak için.
#[macro_export]
macro_rules! k {
    ($key:ident) => {
        $crate::i18n::K::$key
    };
}

#[cfg(test)]
mod tests;
