//! Karıştırma ve sıra kuralları: saf mantık (librespot, ağ, disk yok).
//!
//! `queue.rs` bağlamın çalma sırasını kurarken bu modülden geçirir:
//!
//! - **Hiç çalma** (kara liste): sanatçı adı kara listedeki bir şarkı
//!   sıradan düşer.
//! - **Açık sözlüleri atla**: Spotify'ın "explicit" işaretli şarkıları düşer.
//! - **Aynı şarkının sürümleri**: karıştırılmış bağlamda "Remastered 2011",
//!   "- Live", "(feat. …)", "Radio Edit" gibi eklerden arındırılınca aynı ad +
//!   aynı ana sanatçıya inen şarkılardan yalnız biri kalır (en popüler, eşitse
//!   bağlamda ilk gelen).
//!
//! "Aynı sanatçı art arda gelmesin" kuralı burada değil: dengeli karıştırma
//! (`queue::balanced_shuffle`) zaten yapıyor; bu modül yalnız ona verilecek
//! havuzu süzer. Kullanıcının tek tek seçtiği şarkıya (satıra çift tıklama,
//! "sıraya ekle") kurallar uygulanmaz: o şarkılar sıranın bu havuzundan değil,
//! kullanıcı sırasından ya da `restart`'ın `index`'inden gelir.
//!
//! Eşleştirme **adla** yapılır (şarkı satırı, `TrackRow`, bütün kaynaklarda
//! sanatçı adlarını taşır; sanatçı adresleri yalnız ayrıca istenen bağlarda
//! var). Büyük / küçük harf, boşluk ve tipografik tırnak / tire farkı yok
//! sayılır.

use std::collections::HashMap;

use super::library::TrackRow;

/// Bir şarkının Spotify'dan gelen, sıra kurallarına yarayan özellikleri.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Traits {
    /// Spotify'ın "explicit" işareti.
    pub explicit: bool,
    /// 0–100; bilinmiyorsa 0.
    pub popularity: u8,
}

/// Kara listedeki en çok sanatçı (ayar dosyası şişmesin).
pub const BLOCKED_MAX: usize = 500;

/// Etkin kurallar. `Default`: hiçbir kural yok (sıra eskisi gibi).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    /// Karıştırılmış bağlamda aynı şarkının sürümlerinden birini bırak.
    pub dedupe_versions: bool,
    pub skip_explicit: bool,
    /// `fold`lanmış sanatçı adları (sıralı, tekrarsız).
    blocked: Vec<String>,
}

impl Rules {
    pub fn new<'a>(
        dedupe_versions: bool,
        skip_explicit: bool,
        blocked: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut blocked: Vec<String> = blocked
            .into_iter()
            .map(fold)
            .filter(|n| !n.is_empty())
            .collect();
        blocked.sort();
        blocked.dedup();
        blocked.truncate(BLOCKED_MAX);
        Self {
            dedupe_versions,
            skip_explicit,
            blocked,
        }
    }

    /// Hiçbir şeyi süzmeyen (karıştırmadan bağımsız) kural yok mu?
    pub fn filters_nothing(&self) -> bool {
        !self.skip_explicit && self.blocked.is_empty()
    }

    /// Kuralların hiçbiri açık değil mi?
    pub fn is_empty(&self) -> bool {
        !self.dedupe_versions && self.filters_nothing()
    }

    /// Şarkı özellikleri (açık söz, popülerlik) kuralların işine yarıyor mu?
    pub fn needs_traits(&self) -> bool {
        self.skip_explicit || self.dedupe_versions
    }

    #[cfg(test)]
    pub fn blocked_count(&self) -> usize {
        self.blocked.len()
    }

    /// `artists` ("A, B, C": bir şarkının sanatçıları) kara listeden birini
    /// içeriyor mu? Ad bütün olarak eşleşir ("Tyler, The Creator" iki sanatçı
    /// sayılmaz: listede "tyler" varsa o şarkı yine de düşer, bu bilinen ve
    /// kabul edilen tek istisna; bkz. testler).
    pub fn blocks(&self, artists: &str) -> bool {
        if self.blocked.is_empty() {
            return false;
        }
        let hay = format!(", {}, ", fold(artists));
        self.blocked
            .iter()
            .any(|b| hay.contains(&format!(", {b}, ")))
    }

    /// Bu şarkı (kullanıcı seçmedikçe) çalınmayacak mı?
    pub fn excludes(&self, row: &TrackRow, traits: Option<&Traits>) -> bool {
        (self.skip_explicit && traits.is_some_and(|t| t.explicit)) || self.blocks(&row.artists)
    }
}

// ── Adları eşitleme ─────────────────────────────────────────────────────

/// Karşılaştırma biçimi: küçük harf (Türkçe "İ" doğru), boşluklar tek, eğri
/// tırnaklar düz, uzun tireler "-".
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        let c = match c {
            '\u{2018}' | '\u{2019}' | '\u{02BC}' | '`' | '\u{00B4}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{00A0}' => ' ',
            c => c,
        };
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        match c {
            // `to_lowercase` "İ"yi "i" + birleşik nokta yapar.
            '\u{0130}' => out.push('i'),
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// Bir ek (parantez içi ya da " - " sonrası) şarkıyı DEĞİŞTİRMEYEN bir sürüm
/// notu mu? Muhafazakâr: tanımadığı her şey ayırt edicidir ("Remix",
/// "Acoustic", "Instrumental", "Sped Up", "Part 2", "Demo", "Karaoke", …).
fn is_noise(content: &str) -> bool {
    let tokens: Vec<&str> = content
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let Some(first) = tokens.first() else {
        return false;
    };
    match *first {
        // Katkı sanatçıları: içerik bütünüyle ad.
        "feat" | "ft" | "featuring" | "with" | "prod" => true,
        // Canlı kayıt: "Live", "Live at Wembley, 1986", "Live from …". Yanında
        // başka bir çeşit belirtilmişse ("Live Acoustic") ayrı sürüm.
        "live" => !tokens.iter().any(|t| DISTINCT.contains(t)),
        _ => {
            let mut marker = false;
            for t in &tokens {
                if NOISE_MARKERS.contains(t) {
                    marker = true;
                } else if !(NOISE_FILLER.contains(t) || is_year(t)) {
                    return false;
                }
            }
            marker
        }
    }
}

/// Tek başına sürüm notu sayılan sözcükler.
const NOISE_MARKERS: [&str; 15] = [
    "original",
    "remaster",
    "remastered",
    "remasterizado",
    "radio",
    "single",
    "album",
    "mono",
    "stereo",
    "deluxe",
    "explicit",
    "clean",
    "bonus",
    "edit",
    "version",
];

/// Not içinde yer alabilen ama tek başına not saymayan sözcükler.
const NOISE_FILLER: [&str; 12] = [
    "digital",
    "digitally",
    "edition",
    "expanded",
    "mix",
    "track",
    "the",
    "and",
    "from",
    "cut",
    "re",
    "master",
];

/// "Live" ile birlikte ayrı bir sürüm bildiren sözcükler.
const DISTINCT: [&str; 12] = [
    "acoustic",
    "unplugged",
    "instrumental",
    "remix",
    "karaoke",
    "demo",
    "acapella",
    "cover",
    "reprise",
    "orchestral",
    "piano",
    "session",
];

fn is_year(t: &str) -> bool {
    t.len() == 4 && matches!(t.parse::<u16>(), Ok(1900..=2099))
}

/// Başlığın sürüm eklerinden arındırılmış biçimi (karşılaştırma için).
/// Boş dönebilir (başlık yalnız bir ekten ibaretse).
pub fn normalize_title(title: &str) -> String {
    let t = fold(title);
    let chars: Vec<char> = t.chars().collect();
    // 1. Parantezler: not ise at, değilse içeriği (kırpılmış) bırak.
    let mut head = String::with_capacity(t.len());
    let mut i = 0;
    while i < chars.len() {
        let close = match chars[i] {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            _ => None,
        };
        if let Some(close) = close {
            let open = chars[i];
            let mut depth = 0usize;
            let mut end = None;
            for (j, &c) in chars.iter().enumerate().skip(i) {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(j);
                        break;
                    }
                }
            }
            if let Some(end) = end {
                let inner: String = chars[i + 1..end].iter().collect();
                if is_noise(&inner) {
                    head.push(' ');
                } else {
                    head.push('(');
                    head.push_str(inner.trim());
                    head.push(')');
                }
                i = end + 1;
                continue;
            }
        }
        head.push(chars[i]);
        i += 1;
    }
    // 2. Parantezsiz "feat." / "ft." kuyruğu (sonraki parantez korunur).
    for marker in [" feat. ", " ft. "] {
        if let Some(at) = head.find(marker) {
            let after = head[at + 1..].find('(').map_or(head.len(), |p| at + 1 + p);
            head.replace_range(at..after, " ");
        }
    }
    // 3. " - " ekleri: not olanları at.
    let mut parts = head.split(" - ");
    let mut out = parts.next().unwrap_or("").to_string();
    for part in parts {
        if !is_noise(part) {
            out.push_str(" - ");
            out.push_str(part.trim());
        }
    }
    // 4. Boşluk ve uçlardaki noktalama.
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | ':' | ',' | '.' | '/'))
        .to_string()
}

/// Şarkı adı olarak tek başına anlam taşımayanlar: aynı sanatçının farklı
/// albümlerinde bambaşka kayıtlar olabilir; sürüm sayılmaz.
const GENERIC_TITLES: [&str; 18] = [
    "intro",
    "outro",
    "interlude",
    "skit",
    "untitled",
    "prelude",
    "overture",
    "epilogue",
    "prologue",
    "reprise",
    "instrumental",
    "freestyle",
    "medley",
    "theme",
    "bonus",
    "unknown",
    "track",
    "silence",
];

/// Aynı şarkının sürümlerini eşleyen anahtar: ana sanatçı + eklerden
/// arındırılmış ad. Anahtar çıkarılamıyorsa (sanatçı yok, ad yalnız ek, ad çok
/// genel) `None`: o şarkı hiçbir şeyle eşlenmez.
pub fn song_key(title: &str, artists: &str) -> Option<String> {
    let name = normalize_title(title);
    let letters = name.chars().filter(|c| c.is_alphanumeric()).count();
    if letters < 2
        || name.chars().all(|c| !c.is_alphabetic())
        || GENERIC_TITLES.contains(&name.as_str())
        || GENERIC_TITLES.iter().any(|g| {
            name.strip_prefix(g)
                .is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
        })
    {
        return None;
    }
    let artist = fold(artists.split(", ").next().unwrap_or(""));
    if artist.is_empty() {
        return None;
    }
    Some(format!("{artist}\u{1f}{name}"))
}

/// İki kayıt aynı şarkı olabilir mi (süre sağlaması): biri iki katından
/// uzunsa farklı kayıt sayılır. Süre bilinmiyorsa (0) engellemez.
pub(crate) fn durations_compatible(a: u32, b: u32) -> bool {
    a == 0 || b == 0 || a.max(b) <= a.min(b).saturating_mul(2)
}

// ── Süzme ───────────────────────────────────────────────────────────────

/// Aday bağlam indekslerinden (`candidates`, sırası korunur) kuralların
/// bıraktıklarını döner.
///
/// - `rows` / `traits`: bağlam indeksine göre şarkı satırları ve özellikleri
///   (`traits` boş ya da kısa olabilir: bilinmeyen = açık söz değil, popülerlik 0).
/// - `dedupe`: sürüm eleme bu çağrıda açık mı (yalnız karıştırılmış bağlamda).
/// - `heard`: bu turda zaten çalınmış bağlam indeksleri; bir sürümü çalınmış
///   şarkının öteki sürümleri de düşer.
///
/// Önce kara liste ve açık söz süzülür, sonra sürümler elenir: "açık sözlüleri
/// atla" açıkken popüler olan açık sürüm yerine temiz sürüm kalır.
pub fn select(
    rows: &[TrackRow],
    traits: &[Traits],
    rules: &Rules,
    dedupe: bool,
    heard: &[usize],
    candidates: &[usize],
) -> Vec<usize> {
    let kept: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&i| {
            rows.get(i)
                .is_none_or(|r| !rules.excludes(r, traits.get(i)))
        })
        .collect();
    if !dedupe || !rules.dedupe_versions {
        return kept;
    }

    struct Cluster {
        duration: u32,
        heard: bool,
        /// `kept` içindeki yerleri.
        members: Vec<usize>,
    }
    type Clusters = HashMap<String, Vec<Cluster>>;
    /// Kaydı anahtarın ilk uyumlu kümesine koyar (yoksa yeni küme); kümenin sırası.
    fn place(
        clusters: &mut Clusters,
        key: String,
        duration: u32,
        heard: bool,
        member: Option<usize>,
    ) -> usize {
        let list = clusters.entry(key).or_default();
        let at = list
            .iter()
            .position(|c| durations_compatible(c.duration, duration))
            .unwrap_or_else(|| {
                list.push(Cluster {
                    duration,
                    heard: false,
                    members: Vec::new(),
                });
                list.len() - 1
            });
        list[at].heard |= heard;
        list[at].members.extend(member);
        at
    }
    let mut clusters = Clusters::new();
    // Çalınmışlar önce: kümeleri "duyuldu" diye işaretler.
    for &h in heard {
        if let Some(r) = rows.get(h)
            && let Some(key) = song_key(&r.name, &r.artists)
        {
            place(&mut clusters, key, r.duration_ms, true, None);
        }
    }
    // Her aday: (küme anahtarı, kümedeki sıra) ya da kümesiz.
    let mut slot: Vec<Option<(String, usize)>> = Vec::with_capacity(kept.len());
    for (pos, &i) in kept.iter().enumerate() {
        let key = rows
            .get(i)
            .and_then(|r| song_key(&r.name, &r.artists).map(|k| (k, r.duration_ms)));
        slot.push(key.map(|(k, d)| {
            let at = place(&mut clusters, k.clone(), d, false, Some(pos));
            (k, at)
        }));
    }
    let popularity = |pos: usize| traits.get(kept[pos]).map_or(0, |t| t.popularity);
    kept.iter()
        .enumerate()
        .filter(|&(pos, _)| match &slot[pos] {
            None => true,
            Some((key, at)) => {
                let c = &clusters[key][*at];
                // Çalınmış şarkının sürümü: hiçbiri kalmaz. Yoksa en popüler
                // (eşitse en öndeki) kalır.
                !c.heard
                    && c.members
                        .iter()
                        .copied()
                        .max_by_key(|&m| (popularity(m), std::cmp::Reverse(m)))
                        == Some(pos)
            }
        })
        .map(|(_, &i)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, artists: &str, ms: u32) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{name}{artists}{ms}"),
            name: name.into(),
            artists: artists.into(),
            album: String::new(),
            duration_ms: ms,
            image: String::new(),
            plays: None,
        }
    }

    fn norm(s: &str) -> String {
        normalize_title(s)
    }

    // ── başlık eşitleme ──

    #[test]
    fn strips_remaster_notes_in_every_common_spelling() {
        for t in [
            "Hey Jude - Remastered 2015",
            "Hey Jude (Remastered 2015)",
            "Hey Jude - 2015 Remaster",
            "Hey Jude (2015 Remaster)",
            "Hey Jude - Remastered",
            "Hey Jude [Remastered]",
            "Hey Jude - 2009 Digital Remaster",
            "Hey Jude (Digitally Remastered)",
            "Hey Jude - Remastered Version",
            "Hey Jude - Mono Remaster",
            "Hey Jude (Remastered 2009) [Mono]",
            "Hey Jude - Remastered 2015 - Mono",
            "HEY JUDE - REMASTERED 2015",
        ] {
            assert_eq!(norm(t), "hey jude", "{t}");
        }
    }

    #[test]
    fn strips_live_radio_single_and_credit_notes() {
        for t in [
            "Wonderwall - Live",
            "Wonderwall (Live)",
            "Wonderwall - Live at Wembley Stadium, 1996",
            "Wonderwall (Live from Tokyo)",
            "Wonderwall - Radio Edit",
            "Wonderwall (Radio Edit)",
            "Wonderwall - Single Version",
            "Wonderwall - Album Version",
            "Wonderwall (Edit)",
            "Wonderwall (Explicit)",
            "Wonderwall - Clean",
            "Wonderwall (Bonus Track)",
            "Wonderwall - Deluxe Edition",
            "Wonderwall (feat. Someone Else)",
            "Wonderwall (ft. Someone)",
            "Wonderwall (with Someone)",
            "Wonderwall feat. Someone Else",
            "Wonderwall - feat. Someone",
            "Wonderwall (Original Mix)",
            "Wonderwall (Live) (Remastered 2014)",
        ] {
            assert_eq!(norm(t), "wonderwall", "{t}");
        }
    }

    #[test]
    fn keeps_real_differences_apart() {
        for t in [
            "Wonderwall (Acoustic)",
            "Wonderwall - Acoustic Version",
            "Wonderwall (Live Acoustic)",
            "Wonderwall (Remix)",
            "Wonderwall - Instrumental",
            "Wonderwall (Karaoke Version)",
            "Wonderwall (Sped Up)",
            "Wonderwall (Slowed + Reverb)",
            "Wonderwall, Pt. 2",
            "Wonderwall - Part II",
            "Wonderwall (Demo)",
            "Wonderwall - Extended Mix",
            "Wonderwall (Club Edit)",
            "Wonderwall (From the Motion Picture)",
            "Wonderwall (Piano Version)",
            "Wonderwall (Reprise)",
            "Wonderwall (Cover)",
        ] {
            assert_ne!(norm(t), "wonderwall", "{t} ayrı sürüm sayılmalı");
        }
        assert_ne!(norm("Wonderwall (Remix)"), norm("Wonderwall (Acoustic)"));
        assert_eq!(
            norm("Wonderwall (Remix) - Remastered"),
            norm("Wonderwall (Remix)")
        );
    }

    #[test]
    fn base_titles_that_look_like_notes_are_untouched() {
        // Not yalnız parantez / tire eklerinde aranır; ad kendisi dokunulmaz.
        assert_eq!(norm("Live and Let Die"), "live and let die");
        assert_eq!(norm("Live Forever - Remastered"), "live forever");
        assert_eq!(norm("Feat. Nobody"), "feat. nobody");
        assert_eq!(
            norm("With or Without You - Remastered 2007"),
            "with or without you"
        );
        assert_eq!(norm("Edit"), "edit");
        assert_eq!(norm("Remastered"), "remastered");
        assert_eq!(
            norm("Bohemian Rhapsody - Remastered 2011"),
            "bohemian rhapsody"
        );
        // "With" yalnız parantez içinde katkıdır.
        assert_eq!(norm("Under Pressure (With David Bowie)"), "under pressure");
        assert_eq!(norm("Songs for the Deaf"), "songs for the deaf");
    }

    #[test]
    fn typography_and_case_do_not_matter() {
        assert_eq!(norm("Don’t Stop Believin’"), norm("don't stop believin'"));
        assert_eq!(norm("Hotel California – Remastered"), "hotel california");
        assert_eq!(norm("  Hotel   California  "), "hotel california");
        assert_eq!(norm("İSTANBUL (Remastered)"), "istanbul");
    }

    #[test]
    fn unbalanced_brackets_and_odd_input_do_not_panic() {
        for t in [
            "",
            " ",
            "(",
            ")",
            "((",
            "(Live",
            "Song (Live))",
            "[]",
            "()",
            " - ",
            "- -",
            "Song - ",
            "Song (feat. A (B))",
            "💿",
        ] {
            let _ = norm(t);
        }
        assert_eq!(norm("(Remastered)"), "");
        assert_eq!(norm("Song (feat. A (B))"), "song");
        assert_eq!(norm("Song (Live"), "song (live");
    }

    #[test]
    fn real_world_titles() {
        // Spotify'daki gerçek yazımlar.
        assert_eq!(norm("Smells Like Teen Spirit"), "smells like teen spirit");
        assert_eq!(
            norm("Bohemian Rhapsody - Remastered 2011"),
            norm("Bohemian Rhapsody (Live Aid)")
        );
        assert_eq!(
            norm("Sweet Child O' Mine - Live"),
            norm("Sweet Child O’ Mine")
        );
        assert_eq!(
            norm("Blinding Lights"),
            norm("Blinding Lights - Radio Edit")
        );
        assert_eq!(norm("Shape of You (feat. Ed Sheeran)"), "shape of you");
        assert_eq!(norm("Yesterday - Remastered 2009"), "yesterday");
        assert_eq!(norm("Love Me Do - Mono / Remastered"), "love me do");
        assert_eq!(norm("Fix You - Live from Rome"), "fix you");
        assert_eq!(
            norm("Mr. Brightside (Jacques Lu Cont's Thin White Duke Mix)"),
            "mr. brightside (jacques lu cont's thin white duke mix)"
        );
        assert_eq!(norm("Song 2 - Remastered 2012"), "song 2");
        assert_ne!(norm("Song 2"), norm("Song 1"));
        assert_eq!(norm("Aşk (Remastered 2020)"), "aşk");
    }

    // ── şarkı anahtarı ──

    #[test]
    fn song_key_pairs_primary_artist_with_clean_title() {
        let a = song_key("Hey Jude - Remastered 2015", "The Beatles").unwrap();
        let b = song_key("Hey Jude", "the beatles").unwrap();
        assert_eq!(a, b);
        // Yalnız ilk sanatçı: katkı sanatçısı değişse de aynı şarkı.
        assert_eq!(
            song_key("Despacito", "Luis Fonsi, Daddy Yankee"),
            song_key("Despacito (feat. Daddy Yankee)", "Luis Fonsi")
        );
        // Farklı ana sanatçı: farklı şarkı (cover).
        assert_ne!(
            song_key("Hurt", "Nine Inch Nails"),
            song_key("Hurt", "Johnny Cash")
        );
    }

    #[test]
    fn song_key_refuses_ambiguous_titles() {
        assert_eq!(song_key("Intro", "Drake"), None);
        assert_eq!(song_key("Interlude 2", "Drake"), None);
        assert_eq!(song_key("Untitled", "Kendrick Lamar"), None);
        assert_eq!(song_key("Skit", "Drake"), None);
        assert_eq!(song_key("1", "Beyoncé"), None);
        assert_eq!(song_key("7", "Catfish"), None);
        assert_eq!(song_key("(Remastered)", "Queen"), None);
        assert_eq!(song_key("Hey Jude", ""), None);
        assert!(
            song_key("Intro to Everything", "Drake").is_some(),
            "yalnız tam ad genel sayılır"
        );
    }

    // ── kurallar ──

    #[test]
    fn blocklist_matches_whole_artist_names_case_insensitively() {
        let r = Rules::new(false, false, ["Drake", "  the weeknd "]);
        assert!(r.blocks("Drake"));
        assert!(r.blocks("DRAKE"));
        assert!(r.blocks("Future, Drake"));
        assert!(r.blocks("Drake, Future"));
        assert!(r.blocks("The Weeknd, Daft Punk"));
        assert!(!r.blocks("Drake Bell"), "ön ek eşleşmesi yok");
        assert!(!r.blocks("Drakeo the Ruler"));
        assert!(!r.blocks("Weeknd"));
        assert!(!r.blocks(""));
        assert!(!Rules::default().blocks("Drake"));
        assert_eq!(r.blocked_count(), 2);
        // Tekrarlar ve boşlar atılır.
        assert_eq!(
            Rules::new(false, false, ["A", "a", " ", ""]).blocked_count(),
            1
        );
    }

    #[test]
    fn blocklist_known_comma_name_limitation() {
        // "Tyler, The Creator" ", " ile birleştirildiği için "tyler" listede
        // ise o şarkı da düşer; "tyler, the creator" listedeyken doğru çalışır.
        assert!(Rules::new(false, false, ["Tyler"]).blocks("Tyler, The Creator"));
        let r = Rules::new(false, false, ["Tyler, The Creator"]);
        assert!(r.blocks("Tyler, The Creator"));
        assert!(r.blocks("Frank Ocean, Tyler, The Creator"));
        assert!(!r.blocks("Tyler Childers"));
    }

    #[test]
    fn explicit_filter_needs_a_known_flag() {
        let rows = vec![row("a", "x", 1), row("b", "x", 1), row("c", "x", 1)];
        let traits = vec![
            Traits {
                explicit: true,
                popularity: 0,
            },
            Traits::default(),
        ];
        let rules = Rules::new(false, true, []);
        // 2. şarkının özelliği bilinmiyor: düşmez.
        assert_eq!(
            select(&rows, &traits, &rules, false, &[], &[0, 1, 2]),
            [1, 2]
        );
        assert_eq!(
            select(&rows, &traits, &Rules::default(), false, &[], &[0, 1, 2]),
            [0, 1, 2]
        );
    }

    fn versions() -> (Vec<TrackRow>, Vec<Traits>) {
        let rows = vec![
            row("Hey Jude - Remastered 2015", "The Beatles", 425_000), // 0
            row("Let It Be", "The Beatles", 243_000),                  // 1
            row("Hey Jude", "The Beatles", 431_000),                   // 2
            row("Hey Jude (Live)", "The Beatles", 500_000),            // 3
            row("Hey Jude", "Wilson Pickett", 260_000),                // 4 (cover)
            row("Hey Jude (Acoustic)", "The Beatles", 200_000),        // 5 (ayrı)
        ];
        let pop = |p| Traits {
            explicit: false,
            popularity: p,
        };
        (
            rows,
            vec![pop(60), pop(70), pop(90), pop(10), pop(30), pop(20)],
        )
    }

    #[test]
    fn dedupe_keeps_the_most_popular_version() {
        let (rows, traits) = versions();
        let rules = Rules::new(true, false, []);
        let all: Vec<usize> = (0..rows.len()).collect();
        let kept = select(&rows, &traits, &rules, true, &[], &all);
        // 0, 2, 3 aynı şarkı: en popüler olan 2 kalır. Cover (4), Let It Be (1)
        // ve Acoustic (5) ayrı.
        assert_eq!(kept, [1, 2, 4, 5]);
        // Kapalıysa hiçbiri düşmez.
        assert_eq!(select(&rows, &traits, &rules, false, &[], &all), all);
        assert_eq!(
            select(&rows, &traits, &Rules::default(), true, &[], &all),
            all
        );
    }

    #[test]
    fn dedupe_ties_keep_the_first_and_unknown_popularity_is_first() {
        let (rows, _) = versions();
        let rules = Rules::new(true, false, []);
        let all: Vec<usize> = (0..rows.len()).collect();
        assert_eq!(select(&rows, &[], &rules, true, &[], &all), [0, 1, 4, 5]);
    }

    #[test]
    fn dedupe_drops_versions_of_a_song_already_heard() {
        let (rows, traits) = versions();
        let rules = Rules::new(true, false, []);
        // 2 (Hey Jude) şimdi çalıyor / çalındı: 0 ve 3 kalmaz, hiçbiri.
        let kept = select(&rows, &traits, &rules, true, &[2], &[0, 1, 3, 4, 5]);
        assert_eq!(kept, [1, 4, 5]);
    }

    #[test]
    fn dedupe_runs_after_the_explicit_filter() {
        // Popüler olan açık sürüm, "açıkları atla" açıkken yerini temiz sürüme bırakır.
        let rows = vec![
            row("Song (Explicit)", "A", 200_000),
            row("Song (Clean)", "A", 200_000),
            row("Other", "B", 1),
        ];
        let traits = vec![
            Traits {
                explicit: true,
                popularity: 90,
            },
            Traits {
                explicit: false,
                popularity: 50,
            },
            Traits::default(),
        ];
        let rules = Rules::new(true, true, []);
        assert_eq!(
            select(&rows, &traits, &rules, true, &[], &[0, 1, 2]),
            [1, 2]
        );
        let rules = Rules::new(true, false, []);
        assert_eq!(
            select(&rows, &traits, &rules, true, &[], &[0, 1, 2]),
            [0, 2]
        );
    }

    #[test]
    fn dedupe_respects_duration_sanity() {
        // Aynı ad + sanatçı ama süre iki katından fazla ayrışıyor: farklı kayıt.
        let rows = vec![
            row("Bloom", "Radiohead", 210_000),
            row("Bloom", "Radiohead", 30_000),
        ];
        let rules = Rules::new(true, false, []);
        assert_eq!(select(&rows, &[], &rules, true, &[], &[0, 1]), [0, 1]);
        // Süre bilinmiyorsa eşleşmeyi engellemez.
        let rows = vec![
            row("Bloom", "Radiohead", 0),
            row("Bloom", "Radiohead", 210_000),
        ];
        assert_eq!(select(&rows, &[], &rules, true, &[], &[0, 1]), [0]);
    }

    #[test]
    fn blocked_artists_leave_the_pool_even_when_deduping() {
        let rows = vec![
            row("A", "Drake", 1),
            row("B", "Future, Drake", 1),
            row("C", "Other", 1),
        ];
        let rules = Rules::new(true, false, ["drake"]);
        assert_eq!(select(&rows, &[], &rules, true, &[], &[0, 1, 2]), [2]);
    }

    #[test]
    fn rows_missing_for_an_index_never_exclude() {
        let rows = vec![row("A", "Drake", 1)];
        let rules = Rules::new(true, true, ["drake"]);
        // 5. indeks için satır yok: bilinmeyen şarkı dokunulmaz.
        assert_eq!(select(&rows, &[], &rules, true, &[], &[0, 5]), [5]);
    }

    /// Her koşulda: sonuç adayların sırayı koruyan bir alt kümesi; kurallar
    /// kapalıyken hepsi; kendi başına (yalnız dedupe) ilk aday hiçbir zaman
    /// bütün bir kümeyi yok etmez.
    #[test]
    fn select_is_an_ordered_subset_and_never_empties_a_group() {
        let titles = [
            "Song",
            "Song - Live",
            "Song (Remastered)",
            "Other",
            "Third (feat. X)",
            "Third",
            "Intro",
            "Intro",
        ];
        let artists = ["A", "A", "A", "B", "C", "C", "A", "A"];
        let rows: Vec<TrackRow> = titles
            .iter()
            .zip(artists)
            .enumerate()
            .map(|(i, (t, a))| row(t, a, 200_000 + i as u32))
            .collect();
        let mut state = 7u64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..300 {
            let n = 1 + next() % rows.len();
            let mut cands: Vec<usize> = (0..rows.len()).collect();
            // basit karıştırma
            for i in (1..cands.len()).rev() {
                cands.swap(i, next() % (i + 1));
            }
            cands.truncate(n);
            let traits: Vec<Traits> = (0..rows.len())
                .map(|_| Traits {
                    explicit: next() % 3 == 0,
                    popularity: (next() % 100) as u8,
                })
                .collect();
            let rules = Rules::new(
                true,
                next() % 2 == 0,
                if next() % 2 == 0 { vec!["b"] } else { vec![] },
            );
            let kept = select(&rows, &traits, &rules, true, &[], &cands);
            // alt küme ve sıra korunumu
            let mut it = cands.iter();
            assert!(
                kept.iter().all(|k| it.any(|c| c == k)),
                "{cands:?} → {kept:?}"
            );
            // süzme sonrası boş küme yok: süzülmeyen her anahtar grubundan biri kalır
            let survivors: Vec<usize> = cands
                .iter()
                .copied()
                .filter(|&i| !rules.excludes(&rows[i], traits.get(i)))
                .collect();
            for &s in &survivors {
                let key = song_key(&rows[s].name, &rows[s].artists);
                let same =
                    |i: &usize| key.is_some() && song_key(&rows[*i].name, &rows[*i].artists) == key;
                if key.is_some() {
                    assert!(
                        kept.iter().any(same),
                        "grup tamamen yok oldu: {survivors:?} → {kept:?}"
                    );
                } else {
                    assert!(kept.contains(&s), "anahtarsız şarkı düştü");
                }
            }
            // her anahtardan en çok bir tane (süreler 2x içinde)
            let mut seen = std::collections::HashSet::new();
            for &k in &kept {
                if let Some(key) = song_key(&rows[k].name, &rows[k].artists) {
                    assert!(seen.insert(key), "aynı şarkının iki sürümü kaldı: {kept:?}");
                }
            }
        }
    }

    #[test]
    fn fold_handles_turkish_capitals_and_spacing() {
        assert_eq!(fold("  Sertab   Erener "), "sertab erener");
        assert_eq!(fold("İbrahim Tatlıses"), "ibrahim tatlıses");
        assert_eq!(fold("Beyoncé"), "beyoncé");
        assert_eq!(fold("Guns N’ Roses"), "guns n' roses");
    }

    #[test]
    fn rules_flags() {
        assert!(Rules::default().is_empty());
        assert!(!Rules::new(true, false, []).is_empty());
        assert!(Rules::new(true, false, []).filters_nothing());
        assert!(!Rules::new(false, true, []).filters_nothing());
        assert!(!Rules::new(false, false, ["x"]).filters_nothing());
        assert!(Rules::new(false, true, []).needs_traits());
        assert!(!Rules::new(false, false, ["x"]).needs_traits());
    }
}
