//! i18n testleri: tablo bütünlüğü, çoğul / biçim yardımcıları ve kaynak taraması
//! ("görünen Türkçe yazı koda gömülü kalmasın").

use super::*;

// ── Tablo bütünlüğü ─────────────────────────────────────────────────────────

fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        out.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    out
}

/// Sıralı `{}` ve `{N}` yer tutucularının hangi bağımsız değişkenleri kullandığı.
fn used_args(s: &str) -> Vec<usize> {
    let mut next = 0;
    let mut v: Vec<usize> = placeholders(s)
        .iter()
        .map(|p| {
            if p.is_empty() {
                next += 1;
                next - 1
            } else {
                p.parse()
                    .unwrap_or_else(|_| panic!("yer tutucu {{{p}}} ne boş ne sayı: {s:?}"))
            }
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn every_key_has_both_languages() {
    // `keys!` iki alanı da zorunlu kılar (derleme hatası); burada boş ve kenar
    // boşluklu yazıları yakalarız. Tablo uzunlukları anahtar sayısına eşit olmalı.
    assert_eq!(table::EN.len(), table::ALL.len());
    assert_eq!(table::TR.len(), table::ALL.len());
    for (i, name) in table::NAMES.iter().enumerate() {
        for (lang, s) in [("en", table::EN[i]), ("tr", table::TR[i])] {
            assert!(!s.trim().is_empty(), "{name}: {lang} yazısı boş");
            assert_eq!(
                s,
                s.trim(),
                "{name}: {lang} yazısının başında / sonunda boşluk var"
            );
        }
    }
}

#[test]
fn placeholders_match_across_languages() {
    for (i, name) in table::NAMES.iter().enumerate() {
        let (en, tr) = (table::EN[i], table::TR[i]);
        // Çoğul biçimlerinde her seçenekte aynı yer tutucular olmalı.
        for variant in en.split('|') {
            assert_eq!(
                used_args(variant),
                used_args(tr.split('|').next().unwrap()),
                "{name}: İngilizce {variant:?} ile Türkçe {tr:?} yer tutucuları farklı"
            );
        }
        for variant in tr.split('|') {
            assert_eq!(
                used_args(variant),
                used_args(en.split('|').next().unwrap()),
                "{name}: Türkçe {variant:?} ile İngilizce {en:?} yer tutucuları farklı"
            );
        }
    }
}

#[test]
fn plural_keys_are_well_formed() {
    for (i, name) in table::NAMES.iter().enumerate() {
        for (lang, s) in [("en", table::EN[i]), ("tr", table::TR[i])] {
            let n = s.matches('|').count();
            assert!(n <= 1, "{name}: {lang} yazısında birden çok '|': {s:?}");
            if n == 1 {
                let (one, many) = s.split_once('|').unwrap();
                assert!(
                    one.contains("{}") && many.contains("{}"),
                    "{name}: çoğul seçeneklerde `{{}}` yok: {s:?}"
                );
            }
        }
    }
}

#[test]
fn english_text_has_no_turkish_only_letters() {
    // Türkçe'ye özgü harfler (ğ ş ı İ) İngilizce yazıda ancak "Türkçe" gibi özel adlarda olur.
    for (i, name) in table::NAMES.iter().enumerate() {
        let en = table::EN[i];
        if en.contains("Türkçe") {
            continue;
        }
        assert!(
            !en.chars().any(|c| "ğĞşŞıİ".contains(c)),
            "{name}: İngilizce yazıda Türkçe harf: {en:?}"
        );
    }
}

#[test]
fn turkish_text_is_not_pasted_into_english() {
    // Çevrilmeden bırakılan (İngilizcesi Türkçeyle aynı) yazılar yalnız özel ad / simge /
    // iki dilde aynı olan kısa yazı olabilir.
    const SAME_OK: &[&str] = &[
        "Spotify",
        "English",
        "Türkçe",
        "LRCLIB",
        "Dil / Language",
        "Normal",
        "Podcast",
        "Karaoke",
        "AutoMix",
        "Spotify Connect",
    ];
    for (i, name) in table::NAMES.iter().enumerate() {
        let (en, tr) = (table::EN[i], table::TR[i]);
        if en == tr && en.chars().filter(|c| c.is_alphabetic()).count() > 3 {
            assert!(
                SAME_OK.contains(&en)
                    || en
                        .chars()
                        .all(|c| !c.is_alphabetic() || c.is_ascii_uppercase()),
                "{name}: İngilizce ile Türkçe aynı ({en:?}), çevrilmemiş olabilir; gerçekten aynıysa SAME_OK'a ekle"
            );
        }
    }
}

// ── Yardımcılar ─────────────────────────────────────────────────────────────

#[test]
fn fill_replaces_sequential_and_indexed_placeholders() {
    assert_eq!(fill("{} / {}", &[&3, &"a"]), "3 / a");
    assert_eq!(fill("{1} then {0}", &[&"x", &"y"]), "y then x");
    assert_eq!(fill("no placeholders", &[]), "no placeholders");
    // Eksik bağımsız değişken: olduğu gibi kalır (boş yazı değil).
    assert_eq!(fill("a {} b", &[]), "a {} b");
    assert_eq!(fill("açık { kapanmadı", &[&1]), "açık { kapanmadı");
}

#[test]
fn plural_picks_one_or_many() {
    assert_eq!(pick_plural("{} song|{} songs", 1), "{} song");
    assert_eq!(pick_plural("{} song|{} songs", 0), "{} songs");
    assert_eq!(pick_plural("{} song|{} songs", 2), "{} songs");
    assert_eq!(pick_plural("{} şarkı", 1), "{} şarkı");
    assert_eq!(pick_plural("{} şarkı", 5), "{} şarkı");
}

#[test]
fn number_grouping() {
    assert_eq!(group(0, ','), "0");
    assert_eq!(group(999, ','), "999");
    assert_eq!(group(1_000, ','), "1,000");
    assert_eq!(group(1_234_567, '.'), "1.234.567");
    assert_eq!(group(12_345, '.'), "12.345");
}

#[test]
fn windows_language_id_maps_to_language() {
    assert_eq!(lang_from_langid(0x041f), Lang::Tr); // tr-TR
    assert_eq!(lang_from_langid(0x0409), Lang::En); // en-US
    assert_eq!(lang_from_langid(0x0407), Lang::En); // de-DE
    assert_eq!(lang_from_langid(0x0c0c), Lang::En); // fr-CA
}

#[test]
fn test_hook_overrides_preference() {
    assert_eq!(test_override(Some("en")), Some(Lang::En));
    assert_eq!(test_override(Some(" TR ")), Some(Lang::Tr));
    assert_eq!(test_override(Some("de")), None);
    assert_eq!(test_override(None), None);
}

#[test]
fn turkish_casing_handles_dotted_and_dotless_i() {
    with_lang(Lang::Tr, || {
        assert_eq!(lower("İSTANBUL ILIK"), "istanbul ılık");
        assert_eq!(upper("istanbul ılık"), "İSTANBUL ILIK");
    });
    with_lang(Lang::En, || {
        assert_eq!(lower("ISTANBUL"), "istanbul");
        assert_eq!(upper("istanbul"), "ISTANBUL");
    });
}

// ── Kaynak taraması ─────────────────────────────────────────────────────────

/// Taramaya girmeyen dosyalar (yol parçası): çeviri tablosu, geliştirme vitrini
/// (`OPENIFY_GALLERY`, kullanıcıya görünmez), sınama sabitleri.
const SKIP_FILES: &[&str] = &[
    "i18n.rs",
    "i18n/",
    "brand.rs",
    "ui/gallery.rs",
    "fixtures",
    "/edits/live.rs",
    "engine/output/tests.rs",
    "engine/tests.rs",
    "engine/playback/tests.rs",
    "engine/playback/connect_tests.rs",
    "engine/connect/tests.rs",
];

/// Türkçe kalmasına izin verilen yazılar: (dosya parçası, yazının parçası). Her
/// girdi, neden Türkçe kaldığını açıklayan bir gerekçe taşır.
const ALLOW: &[(&str, &str, &str)] = &[
    // Geliştirme / tanı çıktısı, kullanıcıya gösterilmez.
    ("ui/integration.rs", "sistem:", "günlük satırı"),
    (
        "ui/integration.rs",
        "tepsi / ikinci açılış",
        "Op::Show nedeni: günlük etiketi",
    ),
    (
        "ui/integration.rs",
        "gizli açılış",
        "Op::Show nedeni: günlük etiketi",
    ),
    (
        "ui/popups.rs",
        "tepsi menüsü",
        "Op::Show nedeni: günlük etiketi",
    ),
    ("ui/mod.rs", "aşama", "açılış ölçüm günlüğü"),
    (
        "ui/mod.rs",
        "olay kanalı kapandı",
        "EngineStopped nedeni: günlüğe gider, ekranda msg / t! yazısı görünür",
    ),
    // Motor tanı iletileri: hata nedeni olarak taşınır, günlüğe gider; kullanıcıya hiç ham gösterilmez
    // (`explain_error` / `explain_edit_failure` bunları sınıflayıp `t!` yazısı verir).
    (
        "engine/edits.rs",
        "",
        "yazma işlemi hata nedenleri (FailKind'e sınıflanır)",
    ),
    ("engine/related.rs", "", "ağ hata nedenleri (günlük)"),
    (
        "engine/output/device.rs",
        "akış hatası",
        "signal_device_lost nedeni (günlük); kullanıcı DeviceNotice ile t! yazısı görür",
    ),
    (
        "engine/output/writer.rs",
        "ses aygıtı ayrıldı",
        "SinkError nedeni (çalar günlüğü)",
    ),
    (
        "engine/output/writer.rs",
        "çıkış ilerlemiyor",
        "signal_device_lost nedeni (günlük)",
    ),
    (
        "engine/output/writer.rs",
        "ses çıkışı ilerlemiyor",
        "SinkError nedeni (çalar günlüğü)",
    ),
    ("engine/wire.rs", "sınama:", "sahte hata nedeni"),
    (
        "engine/wire.rs",
        "network:",
        "hata önekiyle sınıflanır (`explain_error`)",
    ),
    ("engine/wire.rs", "önbellek yok", "hata nedeni"),
    (
        "engine/mod.rs",
        "network:",
        "hata önekiyle sınıflanır (`explain_error`)",
    ),
    (
        "engine/session.rs",
        "network:",
        "hata önekiyle sınıflanır (`explain_error`); bağlantı kodu `mod.rs`ten taşındı",
    ),
    // `openify connect-spike` (özellik `connect-spike`): sahibinin gerçek hesapla
    // koşturduğu Türkçe konsol onay listesi, uygulama arayüzü değil.
    (
        "connect_spike",
        "",
        "geliştirici / sahip denemesi, terminal çıktısı (cli.rs gibi)",
    ),
    ("engine/mod.rs", "henüz bağlanılmadı", "hata nedeni"),
    (
        "engine/mod.rs",
        "çalınacak şarkı bulunamadı",
        "hata nedeni (günlük)",
    ),
    (
        "engine/explore.rs",
        "Sana özel",
        "ana sayfa akış bölümü başlıkları ekranda gösterilmez (ana sayfa kendi başlığını kullanır)",
    ),
    ("engine/explore.rs", "Karışımların", "bkz. yukarısı"),
    // Terminal (`openify <bağlantı>`) ve kayıt defteri tanı iletileri.
    (
        "cli.rs",
        "",
        "terminalden çalma komutunun hata iletileri (geliştirici aracı)",
    ),
    (
        "autostart.rs",
        "kayıt defteri hatası",
        "günlüğe giden hata nedeni",
    ),
    // Bileşen vitrininin durum adları (Look::name).
    ("ui/components.rs", "Üzerine gelme", "vitrin etiketi"),
    ("ui/components.rs", "Klavye odağı", "vitrin etiketi"),
    // Sahte çalma (OPENIFY_TEST_FAKE_PLAY) sınama sözü: yalnız geliştirme kancasında görünür.
    ("ui/now_playing.rs", "Gece uzun", "sahte söz"),
    ("ui/now_playing.rs", "Bir şarkı çalıyor eski", "sahte söz"),
    ("ui/now_playing.rs", "Sözler akıyor", "sahte söz"),
    ("ui/now_playing.rs", "Kapak tek ışık", "sahte söz"),
    ("ui/now_playing.rs", "Sürükle bırak", "sahte söz"),
    ("ui/now_playing.rs", "Duyduğun an", "sahte söz"),
    ("ui/now_playing.rs", "Ve yeniden başa", "sahte söz"),
    (
        "ui/now_playing.rs",
        "Openify deneme",
        "sahte söz sağlayıcı adı",
    ),
];

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Tek bir yazı değişmezi: satır, içerik, çevreleyen çağrılardan biri günlük / hata
/// bağlamı mı (kullanıcıya gösterilmeyen yazı).
struct Lit {
    line: usize,
    text: String,
    ignored: bool,
}

/// Çevresindeki çağrının adı bunlardan birini içeriyorsa yazı kullanıcıya gösterilmez
/// (günlük, hata zinciri, çökme iletisi, öznitelik).
const IGNORED_CALLS: &[&str] = &[
    "log::",
    "println!",
    "eprintln!",
    "print!",
    "eprint!",
    "panic!",
    "assert",
    "debug_assert",
    ".expect",
    "anyhow!",
    "bail!",
    ".context",
    ".with_context",
    "unreachable!",
    "unimplemented!",
    "todo!",
    "#",
    "env::var",
    "var_os",
];

/// Basit Rust sözcükleyicisi: yorumları atar, yazı değişmezlerini (normal, ham)
/// çevreleyen çağrı bağlamıyla toplar. `#[cfg(test)]` ilk görüldüğü yerden sonrası
/// (dosya sonundaki sınama modülü) taranmaz.
fn literals(src: &str) -> Vec<Lit> {
    let cut = src.find("#[cfg(test)]").unwrap_or(src.len());
    let b: Vec<char> = src[..cut].chars().collect();
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    let mut last_code = String::new();
    let mut skipping = false;
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        // Satır yorumu (`i18n-skip-start` / `i18n-skip-end`: geliştirme vitrini bölgesi)
        if c == '/' && b.get(i + 1) == Some(&'/') {
            let start = i;
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            let comment: String = b[start..i].iter().collect();
            if comment.contains("i18n-skip-start") {
                skipping = true;
            } else if comment.contains("i18n-skip-end") {
                skipping = false;
            }
            continue;
        }
        // Blok yorumu
        if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                if b[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        // Ham yazı: r"..." / r#"..."#
        if c == 'r'
            && matches!(b.get(i + 1), Some('"') | Some('#'))
            && !last_code.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_')
        {
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) == Some(&'"') {
                j += 1;
                let start_line = line;
                let mut text = String::new();
                loop {
                    if j >= b.len() {
                        break;
                    }
                    if b[j] == '"' && (0..hashes).all(|h| b.get(j + 1 + h) == Some(&'#')) {
                        j += 1 + hashes;
                        break;
                    }
                    if b[j] == '\n' {
                        line += 1;
                    }
                    text.push(b[j]);
                    j += 1;
                }
                let ignored = skipping
                    || stack
                        .iter()
                        .any(|s| IGNORED_CALLS.iter().any(|m| s.contains(m)));
                out.push(Lit {
                    line: start_line,
                    text,
                    ignored,
                });
                i = j;
                continue;
            }
        }
        // Normal yazı
        if c == '"' {
            let start_line = line;
            let mut text = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    text.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    text.push(b[i]);
                    i += 1;
                }
            }
            i += 1;
            let ignored = skipping
                || stack
                    .iter()
                    .any(|s| IGNORED_CALLS.iter().any(|m| s.contains(m)));
            out.push(Lit {
                line: start_line,
                text,
                ignored,
            });
            last_code.push('"');
            continue;
        }
        // Karakter değişmezi ('x', '\n', '"') ile ömür ('a) ayrımı
        if c == '\'' {
            if b.get(i + 1) == Some(&'\\') {
                let mut j = i + 2;
                while j < b.len() && b[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            if b.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
        }
        match c {
            '(' | '[' | '{' => {
                let name: String = last_code
                    .chars()
                    .rev()
                    .take_while(|ch| {
                        ch.is_alphanumeric() || matches!(ch, '_' | ':' | '!' | '.' | '#')
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                stack.push(name);
                last_code.clear();
            }
            ')' | ']' | '}' => {
                stack.pop();
                last_code.clear();
            }
            _ => {
                last_code.push(c);
                if last_code.len() > 80 {
                    let cut_at = last_code
                        .char_indices()
                        .rev()
                        .nth(40)
                        .map(|(k, _)| k)
                        .unwrap_or(0);
                    last_code.drain(..cut_at);
                }
            }
        }
        i += 1;
    }
    out
}

fn scanned_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files
        .into_iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if SKIP_FILES
                .iter()
                .any(|s| rel.contains(s) || format!("/{rel}").contains(s))
            {
                return None;
            }
            Some((rel, std::fs::read_to_string(&p).unwrap()))
        })
        .collect()
}

fn has_turkish_letter(s: &str) -> bool {
    s.chars().any(|c| "ğüşıöçİĞÜŞÖÇ".contains(c))
}

#[test]
fn no_raw_turkish_text_in_sources() {
    let mut found = Vec::new();
    for (file, src) in scanned_sources() {
        for lit in literals(&src) {
            if lit.ignored || !has_turkish_letter(&lit.text) {
                continue;
            }
            if ALLOW
                .iter()
                .any(|(f, s, _)| file.contains(f) && lit.text.contains(s))
            {
                continue;
            }
            found.push(format!("src/{file}:{}: \"{}\"", lit.line, lit.text));
        }
    }
    assert!(
        found.is_empty(),
        "Kodda ham Türkçe yazı kalmış (src/i18n/table.rs'e taşı ve t!(..) kullan; günlük ise log:: makrosu içinde olmalı):\n{}",
        found.join("\n")
    );
}

#[test]
fn moved_turkish_strings_do_not_reappear_as_literals() {
    // Türkçe harfsiz yazılar ("Ana sayfa", "Çal"dan başka "Sistem") için: tablodaki
    // her Türkçe yazı, kaynakta olduğu gibi yazılmış bir değişmez olarak bulunmamalı.
    let mut tr_values: Vec<&str> = (0..table::ALL.len())
        .filter(|&i| {
            table::TR[i] != table::EN[i]
                && table::TR[i].chars().count() >= 5
                && !table::TR[i].contains('{')
        })
        .map(|i| table::TR[i])
        .collect();
    tr_values.sort_unstable();
    tr_values.dedup();
    let mut found = Vec::new();
    for (file, src) in scanned_sources() {
        for lit in literals(&src) {
            if lit.ignored
                || ALLOW
                    .iter()
                    .any(|(f, s, _)| file.contains(f) && lit.text.contains(s))
            {
                continue;
            }
            if tr_values.binary_search(&lit.text.as_str()).is_ok() {
                found.push(format!("src/{file}:{}: \"{}\"", lit.line, lit.text));
            }
        }
    }
    assert!(
        found.is_empty(),
        "Çeviri tablosundaki bir yazı kodda ham değişmez olarak duruyor:\n{}",
        found.join("\n")
    );
}

#[test]
fn literal_scanner_understands_log_context_and_comments() {
    let src = r#"
        // yorum: çalıyor
        fn f() {
            log::info!("günlük: çalıyor {}", x);
            let a = "görünen";
            foo(anyhow!("hata: başarısız"));
            let c = '"';
            let r = r"ham şarkı";
        }
    "#;
    let lits = literals(src);
    let visible: Vec<&str> = lits
        .iter()
        .filter(|l| !l.ignored)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(visible, vec!["görünen", "ham şarkı"]);
}

#[test]
fn real_keys_pluralize_and_format_per_language() {
    with_lang(Lang::En, || {
        assert_eq!(crate::tp!(Songs, 1), "1 song");
        assert_eq!(crate::tp!(Songs, 2), "2 songs");
        assert_eq!(crate::tp!(Songs, 1234), "1,234 songs");
        assert_eq!(crate::tp!(ArtistMonthlyListeners, 1), "1 monthly listener");
        assert_eq!(
            crate::tp!(ArtistMonthlyListeners, 12345678),
            "12,345,678 monthly listeners"
        );
        assert_eq!(
            crate::tp!(FailedAddToList, 1, "“A”"),
            "Couldn't add 1 song to “A”"
        );
        assert_eq!(
            crate::tp!(FailedAddToList, 3, "“A”"),
            "Couldn't add 3 songs to “A”"
        );
        assert_eq!(crate::tf!(DurHoursMinutes, 1, 12), "1 hr 12 min");
        assert_eq!(crate::t!(HomeGreetMorning), "Good morning");
        assert_eq!(crate::t!(TrayOpen), "Open Openify");
    });
    with_lang(Lang::Tr, || {
        assert_eq!(crate::tp!(Songs, 1), "1 şarkı");
        assert_eq!(crate::tp!(Songs, 1234), "1.234 şarkı");
        assert_eq!(
            crate::tp!(ArtistMonthlyListeners, 12345678),
            "12.345.678 aylık dinleyici"
        );
        assert_eq!(
            crate::tp!(FailedAddToList, 3, "“A”"),
            "3 şarkı “A” listesine eklenemedi"
        );
        assert_eq!(crate::tf!(DurHoursMinutes, 1, 12), "1 sa 12 dk");
        assert_eq!(crate::t!(TrayOpen), "Openify'ı aç");
    });
}

#[test]
fn brand_name_comes_from_one_place() {
    assert_eq!(crate::brand::APP_NAME, "Openify");
    // Çeviri tablosundaki ad bu sabitten gelir: yazılarda ham "Openify" bulunmaz.
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/i18n/table.rs"),
    )
    .unwrap();
    assert!(
        !raw.contains("Openify"),
        "table.rs'te ham ad var: app_name!() / app_name_acc!() kullan"
    );
}
