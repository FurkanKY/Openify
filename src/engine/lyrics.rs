//! Şarkı sözleri (motor tarafı; arayüz bağlantısı ayrı iş).
//!
//! Kaynaklar, sırayla:
//!   1. Spotify'ın kendi söz servisi (`spclient.get_lyrics`, oturumun kendi
//!      jetonuyla). Kapsamı en iyisi. librespot'un hazır ayrıştırıcısı kelime
//!      (syllable) eşzamanlı yanıtta düşüyor, o yüzden ham JSON'u kendi esnek
//!      yapılarımızla okuyoruz.
//!   2. LRCLIB (`lrclib.net`, ücretsiz, açık): Spotify'da söz yoksa ya da yalnız
//!      eşzamansız söz varsa (satır eşzamanlısı varsa o tercih edilir).
//!
//! Her iki kaynağın HAM yanıtı (ve "bu kaynakta yok" cevabı, süreli) SQLite
//! önbelleğindeki `lyrics` tablosunda tutulur (bkz. `cache.rs`); ayrıştırıcı
//! sonradan iyileşince (ör. kelime eşzamanlı) yeniden indirmek gerekmez.
//!
//! Saat: arayüz `line_at`'e DUYULAN konumu (`audible_position_ms`) vermeli,
//! çalarınkini değil.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use librespot::core::{SpotifyId, SpotifyUri, error::ErrorKind, session::Session};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{TrackInfo, cache::Db};

// ── Model ───────────────────────────────────────────────────────────────

/// Sözün geldiği kaynak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Spotify,
    Lrclib,
}

impl Source {
    fn key(self) -> &'static str {
        match self {
            Source::Spotify => "spotify",
            Source::Lrclib => "lrclib",
        }
    }
}

/// Zaman bilgisi. Kelime eşzamanlı yanıtlar şimdilik satır eşzamanlı olarak
/// okunur (satır başlangıçları geçerli; kelime ayrıntısı atılır, ham yanıt
/// önbellekte durur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Line,
    Unsynced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    /// Satırın başladığı an (şarkı başından, ms). Eşzamansızda hep 0.
    pub start_ms: u32,
    /// Kaynak biliyorsa bitiş anı (başlangıçtan büyükse), yoksa `None`.
    pub end_ms: Option<u32>,
    /// Boş metin: enstrümantal ara / sözün sonu işareti (arayüz karar verir).
    pub text: String,
    /// Bu satırın çevirileri: `Lyrics::translations[i]` dilindeki satır (boş
    /// yazı: o dilde bu satırın çevirisi yok). Uzunluğu `translations` ile aynı.
    pub alts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    pub source: Source,
    pub sync: SyncKind,
    pub lines: Vec<LyricLine>,
    /// Sözü sağlayan (ör. "Musixmatch"); kaynak vermiyorsa `None`.
    pub provider: Option<String>,
    /// Spotify'ın kendi çevirilerinin dilleri (küçük harf etiket: "tr", "en";
    /// bkz. `parse_spotify`). Çeviri her satırda `LyricLine::alts` içinde.
    /// Yoksa boş (LRCLIB'de hiç yok).
    pub translations: Vec<String>,
}

impl Lyrics {
    /// Verilen dilde (`"tr"`, `"en"`) Spotify çevirisi var mı? Sırası döner
    /// (`LyricLine::alts` içindeki yer). Etiket tam ya da ana dilce eşleşir
    /// ("en" ile "en-US").
    pub fn translation_index(&self, lang: &str) -> Option<usize> {
        let primary = |s: &str| s.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        let want = primary(lang);
        (!want.is_empty()).then(|| self.translations.iter().position(|t| primary(t) == want)).flatten()
    }

    /// `audible_ms` anında okunan satır: başlangıcı bu ana eşit ya da önce olan
    /// son satır. İlk satırdan önce (ya da eşzamansız sözde) `None`.
    /// İkili arama: O(log n), kare başına çağrılabilir.
    pub fn line_at(&self, audible_ms: u32) -> Option<usize> {
        if self.sync != SyncKind::Line {
            return None;
        }
        self.lines
            .partition_point(|l| l.start_ms <= audible_ms)
            .checked_sub(1)
    }
}

/// Ham satırlardan son modeli kurar: boşlukları kırpar, sıralar, "eşzamanlı
/// diye gelen ama hepsi 0" yanıtı eşzamansıza çevirir, boşları ayıklar.
/// Hiç metin yoksa `None`.
fn finish(
    source: Source,
    claimed: SyncKind,
    provider: Option<String>,
    translations: Vec<String>,
    mut lines: Vec<LyricLine>,
) -> Option<Lyrics> {
    for l in &mut lines {
        l.text = l.text.trim().to_owned();
        for a in &mut l.alts {
            *a = a.trim().to_owned();
        }
    }
    if lines.iter().all(|l| l.text.is_empty()) {
        return None;
    }
    let synced = claimed == SyncKind::Line
        && (lines.len() == 1 || lines.iter().any(|l| l.start_ms > 0));
    if synced {
        lines.sort_by_key(|l| l.start_ms); // kararlı: eşit anlarda sıra korunur
        for l in &mut lines {
            if l.end_ms.is_some_and(|e| e <= l.start_ms) {
                l.end_ms = None;
            }
        }
        // Baştaki boş satırların anlamı yok (ilk satırdan önce zaten `None`).
        let first = lines.iter().position(|l| !l.text.is_empty()).unwrap_or(0);
        lines.drain(..first);
    } else {
        // Eşzamansız: kıta boşlukları kalsın, art arda boşlar tek olsun,
        // baştaki ve sondaki boşlar gitsin.
        let mut out: Vec<LyricLine> = Vec::with_capacity(lines.len());
        for mut l in lines {
            l.start_ms = 0;
            l.end_ms = None;
            if l.text.is_empty() && out.last().is_none_or(|p| p.text.is_empty()) {
                continue;
            }
            out.push(l);
        }
        while out.last().is_some_and(|l| l.text.is_empty()) {
            out.pop();
        }
        lines = out;
    }
    Some(Lyrics {
        source,
        sync: if synced { SyncKind::Line } else { SyncKind::Unsynced },
        lines,
        provider,
        translations,
    })
}

// ── Spotify yanıtı (esnek ayrıştırma) ───────────────────────────────────

/// Açık küme: bilmediğimiz bir değer gelirse ayrıştırma düşmez.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
enum SyncType {
    #[serde(rename = "UNSYNCED")]
    Unsynced,
    #[serde(rename = "LINE_SYNCED")]
    LineSynced,
    #[serde(rename = "SYLLABLE_SYNCED")]
    SyllableSynced,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct SpotifyEnvelope {
    lyrics: Option<SpotifyLyrics>,
}

#[derive(Deserialize)]
struct SpotifyLyrics {
    #[serde(rename = "syncType", default)]
    sync_type: Option<SyncType>,
    #[serde(default)]
    lines: Option<Vec<SpotifyLine>>,
    #[serde(rename = "providerDisplayName", default)]
    provider_display_name: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    /// Spotify'ın çevirileri: `[{ "language": "en", "lines": ["…", …] }]`,
    /// satırlar asıl `lines` ile aynı sırada. Anahtar adı bilgiye dayanır
    /// (`alternatives`; `translations` de kabul): canlı doğrulanmadı.
    #[serde(default, alias = "translations")]
    alternatives: Option<Vec<SpotifyAlternative>>,
}

#[derive(Deserialize)]
struct SpotifyAlternative {
    #[serde(default, alias = "lang", alias = "languageCode")]
    language: Option<String>,
    #[serde(default)]
    lines: Option<Vec<AlternativeLine>>,
}

/// Çeviri satırı düz yazı ya da `{ "words": … }` olabilir.
#[derive(Deserialize)]
#[serde(untagged)]
enum AlternativeLine {
    Text(String),
    Object {
        #[serde(default, alias = "text", alias = "line")]
        words: Option<String>,
    },
    Other(serde::de::IgnoredAny),
}

impl AlternativeLine {
    fn into_text(self) -> String {
        match self {
            Self::Text(t) => t,
            Self::Object { words } => words.unwrap_or_default(),
            Self::Other(_) => String::new(),
        }
    }
}

/// Bir yanıtta en çok bu kadar çeviri dili tutulur.
const MAX_TRANSLATIONS: usize = 4;

#[derive(Deserialize)]
struct SpotifyLine {
    #[serde(rename = "startTimeMs", default, deserialize_with = "lenient_ms")]
    start_ms: Option<u64>,
    #[serde(rename = "endTimeMs", default, deserialize_with = "lenient_ms")]
    end_ms: Option<u64>,
    #[serde(default)]
    words: Option<String>,
    /// Kelime eşzamanlı yanıtlarda var; şimdilik okunmuyor (düşmemesi yeter).
    #[serde(default)]
    #[allow(dead_code)]
    syllables: Option<Value>,
}

/// Milisaniye sayı, yazı ("1230"), kesirli ya da boş (null) gelebilir.
fn lenient_ms<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Num {
        U(u64),
        I(i64),
        F(f64),
        S(String),
        Other(serde::de::IgnoredAny),
    }
    let from_f = |v: f64| (v.is_finite() && v >= 0.0).then(|| v.round() as u64);
    Ok(match Num::deserialize(d)? {
        Num::U(v) => Some(v),
        Num::I(v) => u64::try_from(v).ok(),
        Num::F(v) => from_f(v),
        Num::S(s) => {
            let s = s.trim();
            s.parse::<u64>().ok().or_else(|| s.parse::<f64>().ok().and_then(from_f))
        }
        Num::Other(_) => None,
    })
}

fn to_u32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Spotify'ın ham yanıtından sözü çıkarır. Bozuk ya da sözsüz yanıt: `None`.
pub fn parse_spotify(raw: &str) -> Option<Lyrics> {
    let envelope: SpotifyEnvelope = serde_json::from_str(raw).ok()?;
    let l = envelope.lyrics?;
    let mut lines: Vec<LyricLine> = l
        .lines?
        .into_iter()
        .map(|line| LyricLine {
            start_ms: line.start_ms.map_or(0, to_u32),
            end_ms: line.end_ms.map(to_u32),
            text: line.words.unwrap_or_default(),
            alts: Vec::new(),
        })
        .collect();
    let translations = attach_translations(&mut lines, l.alternatives.unwrap_or_default());
    let claimed = match l.sync_type {
        Some(SyncType::Unsynced) => SyncKind::Unsynced,
        // Tür yoksa / bilinmiyorsa: zamanlar işe yarıyorsa `finish` eşzamanlı sayar.
        _ => SyncKind::Line,
    };
    let provider = l
        .provider_display_name
        .or(l.provider)
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty());
    finish(Source::Spotify, claimed, provider, translations, lines)
}

/// Çevirileri satırlara işler (her satırın `alts`'ı, dönen dil listesiyle aynı
/// sırada). Yalnız satır sayısı asıl sözle aynı olan, dili belli, en az bir
/// satırı asıl metinden farklı olan çeviri alınır; gerisi (eksik ya da kaymış
/// satır sayısı: hizayı tutturamayız) atılır. Aynı dil bir kez.
fn attach_translations(lines: &mut [LyricLine], alternatives: Vec<SpotifyAlternative>) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    let mut columns: Vec<Vec<String>> = Vec::new();
    for alt in alternatives {
        let language = alt.language.map(|l| l.trim().to_ascii_lowercase()).unwrap_or_default();
        let Some(texts) = alt.lines else { continue };
        if language.is_empty() || texts.len() != lines.len() || languages.contains(&language) {
            continue;
        }
        let texts: Vec<String> = texts.into_iter().map(|t| t.into_text().trim().to_owned()).collect();
        let differs = texts.iter().zip(lines.iter()).any(|(t, l)| !t.is_empty() && t != l.text.trim());
        if !differs {
            continue;
        }
        languages.push(language);
        columns.push(texts);
        if languages.len() == MAX_TRANSLATIONS {
            break;
        }
    }
    for (i, line) in lines.iter_mut().enumerate() {
        line.alts = columns.iter().map(|c| c[i].clone()).collect();
    }
    languages
}

// ── LRC / LRCLIB ────────────────────────────────────────────────────────

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx`, `mm:ss:xx` → ms.
fn parse_timestamp(tag: &str) -> Option<u64> {
    let (min, rest) = tag.split_once(':')?;
    if min.is_empty() || !min.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (sec, frac) = match rest.find(['.', ':']) {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    if sec.is_empty() || !sec.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Kesir: 1 hane onda bir, 2 hane yüzde bir, 3+ hane binde bir.
    let mut ms = 0u64;
    let mut digits = frac.bytes().take(3).map(|b| u64::from(b - b'0')).collect::<Vec<_>>();
    digits.resize(3, 0);
    for d in digits {
        ms = ms * 10 + d;
    }
    Some(min.parse::<u64>().ok()? * 60_000 + sec.parse::<u64>().ok()? * 1000 + ms)
}

/// Gelişmiş LRC'deki kelime işaretlerini (`<00:12.34>`) metinden atar.
fn strip_word_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i..];
        match tail.find('>') {
            Some(j) if parse_timestamp(&tail[1..j]).is_some() => {
                out.push_str(&rest[..i]);
                rest = &tail[j + 1..];
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// LRC metnini satırlara çevirir: çoklu zaman damgası (`[00:10.00][01:20.00]sözler`)
/// her an için bir satır üretir, `[offset:±ms]` uygulanır, `[ar:..]` gibi
/// künyeler ve zaman damgasız satırlar atılır. Başlangıca göre sıralı döner.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut offset_ms: i64 = 0;
    let mut stamped: Vec<(u64, String)> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim_start_matches('\u{feff}').trim();
        let mut times: Vec<u64> = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            if let Some(t) = parse_timestamp(tag) {
                times.push(t);
            } else if times.is_empty()
                && let Some((key, value)) = tag.split_once(':')
                && key.bytes().all(|b| b.is_ascii_alphabetic())
                && !key.is_empty()
            {
                // Künye satırı: yalnız `offset` işe yarar.
                if key.eq_ignore_ascii_case("offset") {
                    offset_ms = value.trim().parse().unwrap_or(0);
                }
                rest = "";
                break;
            } else {
                break; // "[Nakarat]" gibi: metnin başı
            }
            rest = stripped[end + 1..].trim_start();
        }
        if times.is_empty() {
            continue;
        }
        let text = strip_word_tags(rest).trim().to_owned();
        for t in times {
            stamped.push((t, text.clone()));
        }
    }
    let mut out: Vec<LyricLine> = stamped
        .into_iter()
        .map(|(t, text)| {
            let t = i64::try_from(t).unwrap_or(i64::MAX).saturating_sub(offset_ms).max(0);
            LyricLine { start_ms: u32::try_from(t).unwrap_or(u32::MAX), end_ms: None, text, alts: Vec::new() }
        })
        .collect();
    out.sort_by_key(|l| l.start_ms);
    out
}

/// LRCLIB `get` / `search` kaydı; esnek.
#[derive(Deserialize)]
struct LrclibRecord {
    #[serde(default)]
    instrumental: Option<bool>,
    #[serde(rename = "plainLyrics", default)]
    plain: Option<String>,
    #[serde(rename = "syncedLyrics", default)]
    synced: Option<String>,
}

/// LRCLIB'in ham kaydından sözü çıkarır: önce `syncedLyrics`, yoksa
/// `plainLyrics` (eşzamansız). Enstrümantal / boş: `None`.
pub fn parse_lrclib(raw: &str) -> Option<Lyrics> {
    let rec: LrclibRecord = serde_json::from_str(raw).ok()?;
    if rec.instrumental == Some(true) {
        return None;
    }
    if let Some(synced) = rec.synced.as_deref().filter(|s| !s.trim().is_empty()) {
        let lines = parse_lrc(synced);
        if let Some(l) = finish(Source::Lrclib, SyncKind::Line, None, Vec::new(), lines) {
            if l.sync == SyncKind::Line {
                return Some(l);
            }
        }
    }
    let plain = rec.plain.as_deref().filter(|s| !s.trim().is_empty())?;
    let lines = plain
        .lines()
        .map(|t| LyricLine { start_ms: 0, end_ms: None, text: t.to_owned(), alts: Vec::new() })
        .collect();
    finish(Source::Lrclib, SyncKind::Unsynced, None, Vec::new(), lines)
}

// ── Önbellek + akış ─────────────────────────────────────────────────────

/// Spotify'da "yok" cevabı bu kadar süre hatırlanır (söz sonradan eklenebilir).
const TTL_NONE_SPOTIFY: i64 = 7 * 86_400;
/// LRCLIB topluluk kaynağı: daha sık eklenir.
const TTL_NONE_LRCLIB: i64 = 3 * 86_400;
/// Ağ / hız sınırı gibi geçici hatalar: kısa, yoksa bir ağ kesintisi günlerce
/// söz yok demek olurdu; ama her çalışta yeniden de denenmesin.
const TTL_TRANSIENT: i64 = 120;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Bir kaynağın cevabı.
enum Fetched {
    /// Ham yanıt (söz var gibi).
    Body(String),
    /// Kaynak "yok" dedi (404 / boş).
    Nothing,
    /// Geçici hata (ağ, 429, 5xx): kısa süreli "yok" olarak hatırlanır.
    Failed,
}

/// Bir kaynağı önce önbellekten, yoksa `fetch` ile çözer ve sonucu yazar.
async fn from_source<F, Fut>(
    db: &Db,
    uri: &str,
    source: Source,
    now: i64,
    none_ttl: i64,
    parse: fn(&str) -> Option<Lyrics>,
    fetch: F,
) -> Option<Lyrics>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Fetched>,
{
    match db.lyrics_row(uri, source.key()) {
        Ok(Some(row)) => match row.body {
            Some(body) => {
                if let Some(l) = parse(&body) {
                    return Some(l);
                }
                // Bozuk kayıt: yeniden indir.
            }
            None if row.expires_at.is_some_and(|e| e > now) => return None,
            None => {}
        },
        Ok(None) => {}
        Err(e) => log::warn!("Söz önbelleği okunamadı: {e:#}"),
    }
    let save = |body: Option<&str>, expires_at: Option<i64>| {
        if let Err(e) = db.save_lyrics(uri, source.key(), body, expires_at, now) {
            log::warn!("Söz önbelleğe yazılamadı: {e:#}");
        }
    };
    match fetch().await {
        Fetched::Body(body) => match parse(&body) {
            Some(l) => {
                save(Some(&body), None);
                Some(l)
            }
            // Yanıt geldi ama içinde söz yok (enstrümantal, boş).
            None => {
                save(None, Some(now + none_ttl));
                None
            }
        },
        Fetched::Nothing => {
            save(None, Some(now + none_ttl));
            None
        }
        Fetched::Failed => {
            save(None, Some(now + TTL_TRANSIENT));
            None
        }
    }
}

/// Spotify, sonra gerekirse LRCLIB. Satır eşzamanlı söz her zaman
/// eşzamansızdan üstün; Spotify'ın eşzamansızı, LRCLIB'de eşzamanlı yoksa kalır.
async fn resolve<S, SFut, L, LFut>(db: &Db, uri: &str, now: i64, spotify: S, lrclib: L) -> Option<Lyrics>
where
    S: FnOnce() -> SFut,
    SFut: Future<Output = Fetched>,
    L: FnOnce() -> LFut,
    LFut: Future<Output = Fetched>,
{
    let sp = from_source(db, uri, Source::Spotify, now, TTL_NONE_SPOTIFY, parse_spotify, spotify).await;
    if sp.as_ref().is_some_and(|l| l.sync == SyncKind::Line) {
        return sp;
    }
    let lr = from_source(db, uri, Source::Lrclib, now, TTL_NONE_LRCLIB, parse_lrclib, lrclib).await;
    match (sp, lr) {
        (_, Some(l)) if l.sync == SyncKind::Line => Some(l),
        (Some(s), _) => Some(s),
        (None, l) => l,
    }
}

/// Aynı şarkı için eşzamanlı iki istek (ör. önceden getirme + gerçek istek)
/// ağa iki kez gitmesin: ikincisi birincisi bitince önbellekten okur.
fn inflight_lock(uri: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    // Kimse beklemeyen girdileri at (harita şişmesin).
    map.retain(|_, v| Arc::strong_count(v) > 1);
    map.entry(uri.to_owned()).or_default().clone()
}

/// Bir şarkının sözü: önbellek → Spotify → LRCLIB. Hiçbirinde yoksa `None`.
/// `album`: LRCLIB eşleşmesini keskinleştirir (bilinmiyorsa `None`).
/// Hata vermez: hepsi "yok" sayılır ve (süreli) hatırlanır.
pub async fn get(
    session: &Session,
    db: &Db,
    track: &TrackInfo,
    album: Option<&str>,
) -> Option<Arc<Lyrics>> {
    let SpotifyUri::Track { id } = SpotifyUri::from_uri(&track.uri).ok()? else {
        return None; // bölüm vb.
    };
    let lock = inflight_lock(&track.uri);
    let _guard = lock.lock().await;
    let lyrics = resolve(
        db,
        &track.uri,
        now_secs(),
        || fetch_spotify(session, id),
        || fetch_lrclib(track, album),
    )
    .await?;
    Some(Arc::new(lyrics))
}

/// Sıradaki şarkının sözünü önceden önbelleğe alır (sonuç atılır; çalışa
/// geçilince `get` ağa çıkmadan okur).
pub async fn prefetch(session: &Session, db: &Db, track: &TrackInfo, album: Option<&str>) {
    let found = get(session, db, track, album).await;
    log::debug!(
        "Söz önceden alındı: {} → {:?}",
        track.uri,
        found.as_ref().map(|l| (l.source, l.sync))
    );
}

async fn fetch_spotify(session: &Session, id: SpotifyId) -> Fetched {
    match session.spclient().get_lyrics(&id).await {
        Ok(bytes) if bytes.is_empty() => Fetched::Nothing,
        Ok(bytes) => match String::from_utf8(bytes.to_vec()) {
            Ok(text) => Fetched::Body(text),
            Err(_) => Fetched::Nothing,
        },
        Err(e) if e.kind == ErrorKind::NotFound => Fetched::Nothing,
        Err(e) => {
            log::debug!("Spotify sözü alınamadı: {e}");
            Fetched::Failed
        }
    }
}

// ── LRCLIB istemcisi ────────────────────────────────────────────────────

const LRCLIB: &str = "https://lrclib.net/api";
const USER_AGENT: &str = "Openify/0.1 (personal, https://github.com/FurkanKY/Openify)";
/// LRCLIB süre eşleşmesi ±2 sn.
const DURATION_SLACK_S: f64 = 2.0;
/// İstekler arası en az bekleme (sınır ~90 istek/dk'nın çok altında kal).
const MIN_SPACING: Duration = Duration::from_millis(400);
/// 429'da en çok bu kadar bekleyip yeniden dene; daha uzun istenirse vazgeç.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: usize = 3;

#[derive(Default)]
struct Gate {
    next_slot: Option<Instant>,
    blocked_until: Option<Instant>,
}

fn gate() -> &'static Mutex<Gate> {
    static GATE: OnceLock<Mutex<Gate>> = OnceLock::new();
    GATE.get_or_init(Default::default)
}

fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(10))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .map_err(|e| log::warn!("LRCLIB istemcisi kurulamadı: {e}"))
                .ok()
        })
        .as_ref()
}

/// 0..=30 % rastgelelik (rastgele sayı üreteci bağımlılığı eklememek için
/// sistem saatinin alt saniye kısmı).
fn jitter_factor() -> f64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    1.0 + f64::from(nanos % 1000) / 1000.0 * 0.3
}

/// 429'daki bekleme: `Retry-After` (sn) + en çok %30 rastgelelik; başlık yoksa
/// ya da anlaşılmazsa 5 sn.
fn retry_wait(header: Option<&str>, jitter: f64) -> Duration {
    let secs = header.and_then(|h| h.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s >= 0.0);
    Duration::from_secs_f64(secs.unwrap_or(5.0) * jitter)
}

/// Bir LRCLIB isteği: `Ok(Some(gövde))`, `Ok(None)` = 404, `Err(())` = geçici hata.
async fn lrclib_request(path: &str, params: &[(&str, String)]) -> Result<Option<String>, ()> {
    let client = client().ok_or(())?;
    for attempt in 0..MAX_ATTEMPTS {
        // Önceki 429'dan kalan engel / istekler arası aralık.
        let wait_until = {
            let mut g = gate().lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if g.blocked_until.is_some_and(|b| b > now) {
                return Err(());
            }
            let at = g.next_slot.map_or(now, |s| s.max(now));
            g.next_slot = Some(at + MIN_SPACING);
            at
        };
        tokio::time::sleep_until(wait_until.into()).await;

        let response = match client.get(format!("{LRCLIB}/{path}")).query(params).send().await {
            Ok(r) => r,
            Err(e) => {
                log::debug!("LRCLIB isteği başarısız: {e}");
                return Err(());
            }
        };
        let status = response.status();
        if status.is_success() {
            return response.text().await.map(Some).map_err(|_| ());
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let header = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok());
            let wait = retry_wait(header, jitter_factor());
            log::info!("LRCLIB 429, {:.1} sn bekleniyor", wait.as_secs_f32());
            if wait > MAX_RETRY_WAIT || attempt + 1 == MAX_ATTEMPTS {
                // Uzun bekleme istendi: herkes için bu süre boyunca susalım.
                let mut g = gate().lock().unwrap_or_else(|e| e.into_inner());
                g.blocked_until = Some(Instant::now() + wait.min(Duration::from_secs(120)));
                return Err(());
            }
            tokio::time::sleep(wait).await;
            continue;
        }
        log::debug!("LRCLIB {status}");
        return Err(());
    }
    Err(())
}

/// Parantez içi katkı / sürüm notlarını ve " - Remastered 2011" gibi
/// sonekleri atar (LRCLIB'deki adlar genelde sade).
pub fn clean_title(name: &str) -> String {
    const NOISE: [&str; 11] = [
        "remaster", "version", "mix", "edit", "live", "mono", "stereo", "deluxe", "bonus", "single", "feat",
    ];
    let mut out = String::with_capacity(name.len());
    let mut depth = 0u32;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let mut out = out.trim().to_owned();
    if let Some((head, tail)) = out.split_once(" - ") {
        let t = tail.to_lowercase();
        if NOISE.iter().any(|n| t.contains(n)) {
            out = head.trim().to_owned();
        }
    }
    if out.is_empty() { name.trim().to_owned() } else { out }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LrclibQuery {
    track: String,
    artist: String,
    album: Option<String>,
}

/// Denenecek `get` sorguları, en kesinden gevşeğe (en çok 3).
fn lrclib_queries(track: &TrackInfo, album: Option<&str>) -> Vec<LrclibQuery> {
    let primary = track.artists.split(", ").next().unwrap_or("").trim().to_owned();
    let cleaned = clean_title(&track.name);
    let album = album.map(str::trim).filter(|a| !a.is_empty()).map(str::to_owned);
    let mut out: Vec<LrclibQuery> = Vec::new();
    let mut add = |q: LrclibQuery| {
        if !out.contains(&q) {
            out.push(q);
        }
    };
    add(LrclibQuery { track: track.name.clone(), artist: primary.clone(), album: album.clone() });
    if album.is_some() {
        add(LrclibQuery { track: track.name.clone(), artist: primary.clone(), album: None });
    }
    add(LrclibQuery { track: cleaned, artist: primary, album: None });
    out
}

fn duration_s(track: &TrackInfo) -> Option<f64> {
    (track.duration_ms > 0).then(|| f64::from(track.duration_ms) / 1000.0)
}

/// `search` sonuçlarından süresi ±2 sn içinde olan en iyi kaydı seçer
/// (eşzamanlı olan önce, sonra süreye en yakın). Ham kaydı döner.
fn pick_search_result(results: &str, want_s: Option<f64>) -> Option<String> {
    let records: Vec<Value> = serde_json::from_str(results).ok()?;
    let usable = |r: &&Value, field: &str| {
        r.get(field).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty())
    };
    let delta = |r: &Value| match (want_s, r.get("duration").and_then(Value::as_f64)) {
        (Some(w), Some(d)) => (d - w).abs(),
        (Some(_), None) => f64::INFINITY,
        (None, _) => 0.0,
    };
    let best = |field: &str| {
        records
            .iter()
            .filter(|r| r.get("instrumental").and_then(Value::as_bool) != Some(true))
            .filter(|r| usable(r, field) && delta(r) <= DURATION_SLACK_S)
            .min_by(|a, b| delta(a).total_cmp(&delta(b)))
    };
    // Süre bilinmiyorsa belirsiz eşleşmeyi kabul etme.
    want_s?;
    best("syncedLyrics").or_else(|| best("plainLyrics")).map(Value::to_string)
}

async fn fetch_lrclib(track: &TrackInfo, album: Option<&str>) -> Fetched {
    let want_s = duration_s(track);
    let primary = track.artists.split(", ").next().unwrap_or("").trim().to_owned();
    let queries = lrclib_queries(track, album);
    let mut failed = false;
    for q in &queries {
        let mut params = vec![("track_name", q.track.clone()), ("artist_name", q.artist.clone())];
        if let Some(a) = &q.album {
            params.push(("album_name", a.clone()));
        }
        if let Some(d) = want_s {
            params.push(("duration", format!("{}", d.round() as u32)));
        }
        match lrclib_request("get", &params).await {
            Ok(Some(body)) => return Fetched::Body(body),
            Ok(None) => {}
            Err(()) => {
                failed = true;
                break; // hız sınırı / ağ: devam etmenin anlamı yok
            }
        }
    }
    if !failed {
        // Tam eşleşme yok: arama (ad + sanatçı), süresi uyan kayıt.
        let cleaned = clean_title(&track.name);
        let params = [("track_name", cleaned), ("artist_name", primary)];
        match lrclib_request("search", &params).await {
            Ok(Some(body)) => {
                if let Some(rec) = pick_search_result(&body, want_s) {
                    return Fetched::Body(rec);
                }
            }
            Ok(None) => {}
            Err(()) => failed = true,
        }
    }
    if failed { Fetched::Failed } else { Fetched::Nothing }
}

// ── Testler ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const LINE_SYNCED: &str = r#"{
      "lyrics": {
        "syncType": "LINE_SYNCED",
        "lines": [
          {"startTimeMs": "1230", "words": "Birinci satır", "syllables": [], "endTimeMs": "0"},
          {"startTimeMs": "5000", "words": "  İkinci satır  ", "syllables": [], "endTimeMs": "7500"},
          {"startTimeMs": 9000.0, "words": "", "syllables": [], "endTimeMs": null}
        ],
        "provider": "MusixMatch", "providerLyricsId": "123",
        "providerDisplayName": "Musixmatch", "language": "tr"
      },
      "colors": {"background": -8421505, "text": -16777216, "highlightText": -1},
      "hasVocalRemoval": false
    }"#;

    const UNSYNCED: &str = r#"{"lyrics": {"syncType": "UNSYNCED", "lines": [
        {"startTimeMs": "0", "words": "Satır bir", "syllables": [], "endTimeMs": "0"},
        {"startTimeMs": "0", "words": "", "syllables": [], "endTimeMs": "0"},
        {"startTimeMs": "0", "words": "", "syllables": [], "endTimeMs": "0"},
        {"startTimeMs": "0", "words": "Satır iki", "syllables": [], "endTimeMs": "0"},
        {"startTimeMs": "0", "words": "", "syllables": [], "endTimeMs": "0"}
      ], "provider": "Musixmatch"}}"#;

    /// Kelime eşzamanlı: düşmemeli, satır eşzamanlı okunmalı.
    const SYLLABLE: &str = r#"{"lyrics": {"syncType": "SYLLABLE_SYNCED", "lines": [
        {"startTimeMs": "1000", "endTimeMs": "3000", "words": "Hello world",
         "syllables": [{"startTimeMs": "1000", "numChars": "5"}, {"startTimeMs": 2000, "numChars": 6}]},
        {"startTimeMs": "3200", "endTimeMs": "5000", "words": "Again",
         "syllables": [{"startTimeMs": "3200", "numChars": "5"}]}
      ], "someNewField": {"x": [1,2,3]}}}"#;

    #[test]
    fn spotify_line_synced() {
        let l = parse_spotify(LINE_SYNCED).unwrap();
        assert_eq!((l.source, l.sync), (Source::Spotify, SyncKind::Line));
        assert_eq!(l.provider.as_deref(), Some("Musixmatch"));
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[0], LyricLine { start_ms: 1230, end_ms: None, text: "Birinci satır".into(), alts: vec![] });
        assert_eq!(l.lines[1], LyricLine { start_ms: 5000, end_ms: Some(7500), text: "İkinci satır".into(), alts: vec![] });
        assert_eq!(l.lines[2].start_ms, 9000);
        assert_eq!(l.lines[2].text, "");
    }

    #[test]
    fn spotify_unsynced_collapses_blanks() {
        let l = parse_spotify(UNSYNCED).unwrap();
        assert_eq!(l.sync, SyncKind::Unsynced);
        let texts: Vec<_> = l.lines.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["Satır bir", "", "Satır iki"]); // art arda boş tek, sondaki gitti
        assert!(l.lines.iter().all(|x| x.start_ms == 0));
        assert_eq!(l.line_at(50_000), None);
    }

    #[test]
    fn spotify_syllable_synced_does_not_fail() {
        let l = parse_spotify(SYLLABLE).unwrap();
        assert_eq!(l.sync, SyncKind::Line);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].end_ms, Some(3000));
        assert_eq!(l.line_at(2500), Some(0));
        assert_eq!(l.line_at(3200), Some(1));
    }

    #[test]
    fn spotify_lenient_edge_cases() {
        // Bilinmeyen tür + zamanlar var: satır eşzamanlı sayılır.
        let l = parse_spotify(r#"{"lyrics":{"syncType":"WORD_SYNCED_V9","lines":[{"startTimeMs":"10","words":"a"},{"startTimeMs":"20","words":"b"}]}}"#).unwrap();
        assert_eq!(l.sync, SyncKind::Line);
        // Tür yok, zaman yok: eşzamansız.
        let l = parse_spotify(r#"{"lyrics":{"lines":[{"words":"a"},{"words":"b"}]}}"#).unwrap();
        assert_eq!(l.sync, SyncKind::Unsynced);
        // "Eşzamanlı" ama hepsi 0: eşzamansız.
        let l = parse_spotify(r#"{"lyrics":{"syncType":"LINE_SYNCED","lines":[{"startTimeMs":"0","words":"a"},{"startTimeMs":"0","words":"b"}]}}"#).unwrap();
        assert_eq!(l.sync, SyncKind::Unsynced);
        // Sırasız gelen satırlar sıralanır; bitiş <= başlangıç atılır.
        let l = parse_spotify(r#"{"lyrics":{"syncType":"LINE_SYNCED","lines":[{"startTimeMs":"500","endTimeMs":"400","words":"b"},{"startTimeMs":"100","words":"a"}]}}"#).unwrap();
        assert_eq!(l.lines[0].text, "a");
        assert_eq!(l.lines[1].end_ms, None);
        // Baştaki boş satır atılır.
        let l = parse_spotify(r#"{"lyrics":{"syncType":"LINE_SYNCED","lines":[{"startTimeMs":"0","words":""},{"startTimeMs":"900","words":"a"}]}}"#).unwrap();
        assert_eq!(l.lines.len(), 1);
        // Sözsüz / bozuk / boş yanıtlar: None.
        for bad in [
            "",
            "{}",
            "not json",
            r#"{"lyrics":null}"#,
            r#"{"lyrics":{"lines":[]}}"#,
            r#"{"lyrics":{"lines":[{"words":"  "}]}}"#,
            r#"{"lyrics":{"lines":null}}"#,
        ] {
            assert!(parse_spotify(bad).is_none(), "{bad}");
        }
    }

    // ── Spotify çevirisi ────────────────────────────────────────────────

    const TRANSLATED: &str = include_str!("fixtures/lyrics_translated.json");
    const PLAIN: &str = include_str!("fixtures/lyrics_plain.json");

    #[test]
    fn spotify_translations_ride_along_line_by_line() {
        let l = parse_spotify(TRANSLATED).unwrap();
        assert_eq!(l.sync, SyncKind::Line);
        assert_eq!(l.translations, ["en", "tr"]);
        assert_eq!(l.lines.len(), 10);
        assert!(l.lines.iter().all(|x| x.alts.len() == 2), "her satırda her dilin satırı var (boş olabilir)");
        assert_eq!(l.lines[0].text, "La noche es larga y la ciudad en silencio");
        assert_eq!(l.lines[0].alts[0], "The night is long and the city is silent");
        assert_eq!(l.lines[0].alts[1], "Gece uzun, şehir sessiz");
        // Enstrümantal ara: çevirisi de boş, hizayı bozmaz.
        assert_eq!((l.lines[3].text.as_str(), l.lines[3].alts[0].as_str()), ("", ""));
        assert_eq!(l.lines[4].alts[0], "The cover is the only light in the room");
        // Dil eşleşmesi: tam, ana dil, büyük harf; olmayan ya da boş dil yok.
        assert_eq!(l.translation_index("tr"), Some(1));
        assert_eq!(l.translation_index("en"), Some(0));
        assert_eq!(l.translation_index("EN-us"), Some(0));
        assert_eq!(l.translation_index("de"), None);
        assert_eq!(l.translation_index(""), None);
    }

    #[test]
    fn songs_without_a_translation_have_none() {
        let l = parse_spotify(PLAIN).unwrap();
        assert!(l.translations.is_empty());
        assert!(l.lines.iter().all(|x| x.alts.is_empty()));
        assert_eq!(l.translation_index("tr"), None);
        // Alan hiç yoksa (eski yanıtlar) ve LRCLIB'de de aynı.
        let l = parse_spotify(LINE_SYNCED).unwrap();
        assert!(l.translations.is_empty());
        let l = parse_lrclib(r#"{"syncedLyrics":"[00:01.00]a\n[00:02.00]b"}"#).unwrap();
        assert!(l.translations.is_empty() && l.lines.iter().all(|x| x.alts.is_empty()));
    }

    #[test]
    fn misaligned_or_pointless_translations_are_dropped() {
        let body = |alt: &str| {
            format!(
                r#"{{"lyrics":{{"syncType":"LINE_SYNCED","lines":[
                    {{"startTimeMs":"10","words":"uno"}},{{"startTimeMs":"20","words":"dos"}}],
                    "alternatives":{alt}}}}}"#
            )
        };
        // Satır sayısı tutmuyor: hiza kurulamaz, atılır.
        let l = parse_spotify(&body(r#"[{"language":"en","lines":["one"]}]"#)).unwrap();
        assert!(l.translations.is_empty());
        // Dil yok: atılır. Aynı dil ikinci kez: ilki kalır.
        let l = parse_spotify(&body(
            r#"[{"lines":["one","two"]},{"language":"en","lines":["one","two"]},{"language":"EN","lines":["x","y"]}]"#,
        ))
        .unwrap();
        assert_eq!(l.translations, ["en"]);
        assert_eq!(l.lines[1].alts, ["two"]);
        // Asıl metnin aynısı (çeviri değil): atılır.
        let l = parse_spotify(&body(r#"[{"language":"es","lines":["uno","dos"]}]"#)).unwrap();
        assert!(l.translations.is_empty());
        // Çeviri satırı nesne / null olabilir; düşmez.
        let l = parse_spotify(&body(r#"[{"language":"en","lines":[{"words":" one "},null]}]"#)).unwrap();
        assert_eq!(l.translations, ["en"]);
        assert_eq!(l.lines[0].alts, ["one"]);
        assert_eq!(l.lines[1].alts, [""]);
        // Başka anahtar adı (`translations`, `lang`).
        let swapped = body("0").replace(r#""alternatives":0"#, r#""translations":[{"lang":"en","lines":["one","two"]}]"#);
        assert_eq!(parse_spotify(&swapped).unwrap().translations, ["en"]);
        // Alan null ya da bozuksa söz yine de okunur (çevirisiz) ya da yanıt atılır, düşmez.
        assert!(parse_spotify(&body("null")).unwrap().translations.is_empty());
        let _ = parse_spotify(&body(r#""bozuk""#));
        // En çok MAX_TRANSLATIONS dil.
        let many: Vec<String> =
            ["a", "b", "c", "d", "e", "f"].iter().map(|c| format!(r#"{{"language":"{c}","lines":["x","y"]}}"#)).collect();
        let l = parse_spotify(&body(&format!("[{}]", many.join(",")))).unwrap();
        assert_eq!(l.translations.len(), MAX_TRANSLATIONS);
    }

    #[test]
    fn translations_stay_aligned_when_unsynced_blanks_collapse() {
        let l = parse_spotify(
            r#"{"lyrics":{"syncType":"UNSYNCED","lines":[
                {"words":""},{"words":"uno"},{"words":""},{"words":""},{"words":"dos"},{"words":""}],
                "alternatives":[{"language":"en","lines":["", "one", "", "", "two", ""]}]}}"#,
        )
        .unwrap();
        // Baştaki / art arda / sondaki boşlar gitti; çeviri aynı satırlarla gitti.
        let pairs: Vec<(&str, &str)> = l.lines.iter().map(|x| (x.text.as_str(), x.alts[0].as_str())).collect();
        assert_eq!(pairs, [("uno", "one"), ("", ""), ("dos", "two")]);
    }

    #[test]
    fn translation_follows_its_line_when_synced_lines_are_reordered() {
        let l = parse_spotify(
            r#"{"lyrics":{"syncType":"LINE_SYNCED","lines":[
                {"startTimeMs":"500","words":"dos"},{"startTimeMs":"100","words":"uno"}],
                "alternatives":[{"language":"en","lines":["two","one"]}]}}"#,
        )
        .unwrap();
        assert_eq!((l.lines[0].text.as_str(), l.lines[0].alts[0].as_str()), ("uno", "one"));
        assert_eq!((l.lines[1].text.as_str(), l.lines[1].alts[0].as_str()), ("dos", "two"));
    }

    #[test]
    fn line_at_binary_search() {
        let l = parse_spotify(LINE_SYNCED).unwrap();
        assert_eq!(l.line_at(0), None);
        assert_eq!(l.line_at(1229), None);
        assert_eq!(l.line_at(1230), Some(0));
        assert_eq!(l.line_at(4999), Some(0));
        assert_eq!(l.line_at(5000), Some(1));
        assert_eq!(l.line_at(8999), Some(1));
        assert_eq!(l.line_at(9000), Some(2));
        assert_eq!(l.line_at(u32::MAX), Some(2));
        // Aynı başlangıçlı satırlarda sonuncusu.
        let dup = Lyrics {
            source: Source::Lrclib,
            sync: SyncKind::Line,
            provider: None,
            translations: vec![],
            lines: [(0, "a"), (10, "b"), (10, "c"), (20, "d")]
                .map(|(t, s)| LyricLine { start_ms: t, end_ms: None, text: s.into(), alts: vec![] })
                .to_vec(),
        };
        assert_eq!(dup.line_at(10), Some(2));
        assert_eq!(dup.line_at(15), Some(2));
    }

    #[test]
    fn lrc_timestamps() {
        assert_eq!(parse_timestamp("00:12.34"), Some(12_340));
        assert_eq!(parse_timestamp("01:02.5"), Some(62_500));
        assert_eq!(parse_timestamp("01:02.05"), Some(62_050));
        assert_eq!(parse_timestamp("01:02.345"), Some(62_345));
        assert_eq!(parse_timestamp("01:02:34"), Some(62_340));
        assert_eq!(parse_timestamp("03:07"), Some(187_000));
        assert_eq!(parse_timestamp("100:00.00"), Some(6_000_000));
        for bad in ["ar:Artist", "", "1:", ":10", "aa:bb", "00:1x.00", "00:10.x"] {
            assert_eq!(parse_timestamp(bad), None, "{bad}");
        }
    }

    #[test]
    fn lrc_parsing() {
        let lrc = "\u{feff}[ar:Sanatçı]\r\n[ti:Ad]\n[length: 03:20]\n\n\
                   [00:10.00]Birinci\n\
                   [00:20.50][01:30.25] Nakarat <00:21.00>satırı <00:22.00>\n\
                   [Nakarat]\n\
                   [00:40.00][Nakarat] Köşeli metin\n\
                   zaman damgasız satır\n\
                   [01:00]\n\
                   [02:00.5]Son";
        let lines = parse_lrc(lrc);
        let got: Vec<(u32, &str)> = lines.iter().map(|l| (l.start_ms, l.text.as_str())).collect();
        assert_eq!(
            got,
            [
                (10_000, "Birinci"),
                (20_500, "Nakarat satırı"),
                (40_000, "[Nakarat] Köşeli metin"),
                (60_000, ""),
                (90_250, "Nakarat satırı"),
                (120_500, "Son"),
            ]
        );
        assert!(lines.iter().all(|l| l.end_ms.is_none()));
    }

    #[test]
    fn lrc_offset_applies() {
        // +ms: sözler daha erken görünür.
        let lines = parse_lrc("[offset:+500]\n[00:01.00]a\n[00:00.20]b");
        assert_eq!(lines.iter().map(|l| l.start_ms).collect::<Vec<_>>(), [0, 500]);
        let lines = parse_lrc("[offset:-250]\n[00:01.00]a");
        assert_eq!(lines[0].start_ms, 1250);
    }

    #[test]
    fn lrclib_records() {
        let both = r#"{"id":1,"trackName":"x","duration":200.0,"instrumental":false,
            "plainLyrics":"a\nb","syncedLyrics":"[00:01.00]a\n[00:02.50]b"}"#;
        let l = parse_lrclib(both).unwrap();
        assert_eq!((l.source, l.sync, l.lines.len()), (Source::Lrclib, SyncKind::Line, 2));
        assert_eq!(l.lines[1].start_ms, 2500);

        let plain = r#"{"plainLyrics":"bir\n\niki\n","syncedLyrics":null}"#;
        let l = parse_lrclib(plain).unwrap();
        assert_eq!(l.sync, SyncKind::Unsynced);
        assert_eq!(l.lines.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["bir", "", "iki"]);

        // Zaman damgasız "synced": düz metne düşer.
        let weird = r#"{"plainLyrics":"p","syncedLyrics":"zaman damgası yok"}"#;
        assert_eq!(parse_lrclib(weird).unwrap().sync, SyncKind::Unsynced);

        assert!(parse_lrclib(r#"{"instrumental":true,"plainLyrics":"x"}"#).is_none());
        assert!(parse_lrclib(r#"{"plainLyrics":"","syncedLyrics":""}"#).is_none());
        assert!(parse_lrclib("[]").is_none());
        assert!(parse_lrclib("bozuk").is_none());
    }

    #[test]
    fn lrclib_search_pick() {
        let results = r#"[
          {"id":1,"duration":240.0,"plainLyrics":"x","syncedLyrics":"[00:01.00]x"},
          {"id":2,"duration":201.5,"plainLyrics":"y","syncedLyrics":null},
          {"id":3,"duration":199.0,"plainLyrics":"z","syncedLyrics":"[00:01.00]z"},
          {"id":4,"duration":200.2,"plainLyrics":"w","syncedLyrics":"[00:01.00]w","instrumental":true}
        ]"#;
        let pick = pick_search_result(results, Some(200.0)).unwrap();
        assert!(pick.contains("\"id\":3"), "{pick}"); // süresi uyan eşzamanlı
        assert!(pick_search_result(results, Some(100.0)).is_none());
        assert!(pick_search_result(results, None).is_none());
        assert!(pick_search_result("bozuk", Some(1.0)).is_none());
        let only_plain = r#"[{"id":2,"duration":201.5,"plainLyrics":"y","syncedLyrics":""}]"#;
        assert!(pick_search_result(only_plain, Some(200.0)).unwrap().contains("\"id\":2"));
    }

    #[test]
    fn titles_and_queries() {
        assert_eq!(clean_title("Yesterday - Remastered 2009"), "Yesterday");
        assert_eq!(clean_title("Song (feat. Someone) [Live]"), "Song");
        assert_eq!(clean_title("Plain - Part 2"), "Plain - Part 2");
        assert_eq!(clean_title("(Parantez)"), "(Parantez)");
        let t = TrackInfo {
            uri: "spotify:track:x".into(),
            name: "Song (feat. B)".into(),
            artists: "A, B".into(),
            duration_ms: 200_400,
            cover_url: None,
        };
        let q = lrclib_queries(&t, Some("Album"));
        assert_eq!(q.len(), 3);
        assert_eq!(q[0].album.as_deref(), Some("Album"));
        assert_eq!((q[0].artist.as_str(), q[1].album.clone()), ("A", None));
        assert_eq!(q[2].track, "Song");
        assert_eq!(lrclib_queries(&t, None).len(), 2);
        assert_eq!(duration_s(&t), Some(200.4));
    }

    #[test]
    fn retry_after_is_honoured_with_jitter() {
        let w = retry_wait(Some("10"), 1.0);
        assert_eq!(w, Duration::from_secs(10));
        let w = retry_wait(Some(" 10 "), 1.3);
        assert!((w.as_secs_f64() - 13.0).abs() < 1e-6);
        assert_eq!(retry_wait(None, 1.0), Duration::from_secs(5));
        assert_eq!(retry_wait(Some("Wed, 21 Oct 2026 07:28:00 GMT"), 1.0), Duration::from_secs(5));
        for _ in 0..50 {
            let j = jitter_factor();
            assert!((1.0..=1.3).contains(&j), "{j}");
        }
    }

    // ── Önbellek ve akış (sahte kaynaklarla) ──

    fn counted<'a>(
        n: &'a AtomicUsize,
        out: impl Fn() -> Fetched + 'a,
    ) -> impl FnOnce() -> std::future::Ready<Fetched> + 'a {
        move || {
            n.fetch_add(1, Ordering::SeqCst);
            std::future::ready(out())
        }
    }

    #[tokio::test]
    async fn spotify_line_synced_wins_and_is_cached_raw() {
        let db = Db::open_in_memory().unwrap();
        let (sp, lr) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let got = resolve(
            &db,
            "spotify:track:a",
            1000,
            counted(&sp, || Fetched::Body(LINE_SYNCED.into())),
            counted(&lr, || Fetched::Nothing),
        )
        .await
        .unwrap();
        assert_eq!((got.source, got.sync), (Source::Spotify, SyncKind::Line));
        assert_eq!((sp.load(Ordering::SeqCst), lr.load(Ordering::SeqCst)), (1, 0));
        // İkinci çağrıda ağa çıkılmaz; ham yanıt diskte.
        let again = resolve(
            &db,
            "spotify:track:a",
            1001,
            counted(&sp, || Fetched::Failed),
            counted(&lr, || Fetched::Failed),
        )
        .await
        .unwrap();
        assert_eq!(again, got);
        assert_eq!((sp.load(Ordering::SeqCst), lr.load(Ordering::SeqCst)), (1, 0));
        assert_eq!(db.lyrics_row("spotify:track:a", "spotify").unwrap().unwrap().body.as_deref(), Some(LINE_SYNCED));
    }

    #[tokio::test]
    async fn lrclib_synced_beats_spotify_unsynced_and_fallback_works() {
        let lrc = r#"{"plainLyrics":"a","syncedLyrics":"[00:01.00]a"}"#;
        let db = Db::open_in_memory().unwrap();
        let none = AtomicUsize::new(0);
        let got = resolve(
            &db,
            "spotify:track:u",
            0,
            counted(&none, || Fetched::Body(UNSYNCED.into())),
            counted(&none, || Fetched::Body(lrc.into())),
        )
        .await
        .unwrap();
        assert_eq!((got.source, got.sync), (Source::Lrclib, SyncKind::Line));

        // LRCLIB'de de eşzamansız: Spotify'ınki kalır.
        let db = Db::open_in_memory().unwrap();
        let plain = r#"{"plainLyrics":"a"}"#;
        let got = resolve(
            &db,
            "spotify:track:v",
            0,
            counted(&none, || Fetched::Body(UNSYNCED.into())),
            counted(&none, || Fetched::Body(plain.into())),
        )
        .await
        .unwrap();
        assert_eq!(got.source, Source::Spotify);

        // Spotify'da yok → LRCLIB.
        let db = Db::open_in_memory().unwrap();
        let got = resolve(
            &db,
            "spotify:track:w",
            0,
            counted(&none, || Fetched::Nothing),
            counted(&none, || Fetched::Body(plain.into())),
        )
        .await
        .unwrap();
        assert_eq!((got.source, got.sync), (Source::Lrclib, SyncKind::Unsynced));
    }

    #[tokio::test]
    async fn negative_results_are_cached_with_ttl() {
        let db = Db::open_in_memory().unwrap();
        let (sp, lr) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let run = |now: i64, sp_out: fn() -> Fetched, lr_out: fn() -> Fetched| {
            let (db, sp, lr) = (&db, &sp, &lr);
            async move {
                resolve(db, "spotify:track:n", now, counted(sp, sp_out), counted(lr, lr_out)).await
            }
        };
        assert!(run(0, || Fetched::Nothing, || Fetched::Nothing).await.is_none());
        assert_eq!((sp.load(Ordering::SeqCst), lr.load(Ordering::SeqCst)), (1, 1));
        // TTL içinde: ağa çıkılmaz.
        assert!(run(TTL_NONE_LRCLIB - 1, || Fetched::Failed, || Fetched::Failed).await.is_none());
        assert_eq!((sp.load(Ordering::SeqCst), lr.load(Ordering::SeqCst)), (1, 1));
        // LRCLIB'in süresi doldu (3 gün), Spotify'ınki (7 gün) dolmadı.
        assert!(run(TTL_NONE_LRCLIB + 1, || Fetched::Failed, || Fetched::Nothing).await.is_none());
        assert_eq!((sp.load(Ordering::SeqCst), lr.load(Ordering::SeqCst)), (1, 2));
        // İkisi de doldu: ikisine de yeniden sorulur ve bu sefer bulunur.
        let lrc = || Fetched::Body(r#"{"syncedLyrics":"[00:01.00]a"}"#.into());
        let got = run(TTL_NONE_SPOTIFY + 10, || Fetched::Nothing, lrc).await.unwrap();
        assert_eq!(got.source, Source::Lrclib);
        assert_eq!((sp.load(Ordering::SeqCst), lr.load(Ordering::SeqCst)), (2, 3));
    }

    #[tokio::test]
    async fn transient_failures_are_remembered_only_briefly() {
        let db = Db::open_in_memory().unwrap();
        let n = AtomicUsize::new(0);
        let fail = || Fetched::Failed;
        for now in [0, TTL_TRANSIENT - 1] {
            assert!(resolve(&db, "spotify:track:t", now, counted(&n, fail), counted(&n, fail)).await.is_none());
        }
        assert_eq!(n.load(Ordering::SeqCst), 2); // yalnız ilk çağrı (iki kaynak) ağa çıktı
        let _ = resolve(&db, "spotify:track:t", TTL_TRANSIENT + 1, counted(&n, fail), counted(&n, fail)).await;
        assert_eq!(n.load(Ordering::SeqCst), 4); // süre doldu: yeniden denendi
    }

    #[tokio::test]
    async fn instrumental_or_empty_response_is_negative_and_corrupt_cache_refetches() {
        let db = Db::open_in_memory().unwrap();
        let n = AtomicUsize::new(0);
        let inst = || Fetched::Body(r#"{"instrumental":true}"#.into());
        let r = resolve(&db, "spotify:track:i", 0, counted(&n, || Fetched::Nothing), counted(&n, inst)).await;
        assert!(r.is_none());
        let row = db.lyrics_row("spotify:track:i", "lrclib").unwrap().unwrap();
        assert_eq!((row.body, row.expires_at), (None, Some(TTL_NONE_LRCLIB)));

        // Bozuk önbellek kaydı yeniden indirilir.
        db.save_lyrics("spotify:track:c", "spotify", Some("bozuk"), None, 0).unwrap();
        let got = resolve(
            &db,
            "spotify:track:c",
            1,
            counted(&n, || Fetched::Body(LINE_SYNCED.into())),
            counted(&n, || Fetched::Nothing),
        )
        .await;
        assert!(got.is_some());
    }

    // LRCLIB hattının canlı denetimi (Spotify'a hiç gitmez; aynı 3 şarkı).
    //   cargo test --release live_lrclib -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "ağ gerekir"]
    async fn live_lrclib() {
        let cases = [
            ("Kolay Değildir", "Duman", 310_507, Some("Darmaduman")),
            ("Blinding Lights", "The Weeknd", 200_040, Some("After Hours")),
            ("Blinding Lights", "Revelries, Victoria Voss", 176_406, Some("Blinding Lights")),
        ];
        for (i, (name, artists, duration_ms, album)) in cases.into_iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let track = TrackInfo {
                uri: "spotify:track:x".into(),
                name: name.into(),
                artists: artists.into(),
                duration_ms,
                cover_url: None,
            };
            let t0 = Instant::now();
            let fetched = fetch_lrclib(&track, album).await;
            let parsed = match &fetched {
                Fetched::Body(b) => parse_lrclib(b),
                _ => None,
            };
            println!(
                "LRCLIB {artists} — {name}: {} ({} ms) → {:?}",
                match fetched {
                    Fetched::Body(_) => "kayıt var",
                    Fetched::Nothing => "yok",
                    Fetched::Failed => "HATA",
                },
                t0.elapsed().as_millis(),
                parsed.map(|l| (l.sync, l.lines.len())),
            );
        }
    }

    // ── Canlı denetim (elle): kayıtlı girişle en çok 3 şarkı ──
    //
    //   cargo test --release live_lyrics -- --ignored --nocapture
    //
    // Kayıtlı giriş yoksa tarayıcı AÇMAZ, atlar. Müzik çalmaz.
    #[tokio::test]
    #[ignore = "ağ + kayıtlı Spotify girişi gerekir"]
    async fn live_lyrics() {
        if !crate::auth::has_saved() {
            println!("kayıtlı giriş yok, atlandı");
            return;
        }
        let session = crate::auth::connect(None).await.expect("bağlanılamadı");
        let db = Db::open_in_memory().unwrap();
        // OPENIFY_LYRICS_PICKS="uri;uri;uri": Türkçe, popüler İngilizce, sönük.
        // Ad / sanatçı / süre / albüm Spotify'dan okunur (kodlama sorunu yok).
        let picks = std::env::var("OPENIFY_LYRICS_PICKS").unwrap_or_default();
        let picks: Vec<String> = picks.split(';').filter(|s| !s.is_empty()).map(str::to_owned).collect();
        assert!(picks.len() <= 3, "en çok 3 şarkı");
        for (i, uri) in picks.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(Duration::from_secs(8)).await;
            }
            let parsed = SpotifyUri::from_uri(uri).expect("geçersiz adres");
            let meta = librespot::metadata::Track::get(&session, &parsed).await.expect("şarkı bilgisi");
            use librespot::metadata::Metadata;
            let track = TrackInfo {
                uri: uri.clone(),
                name: meta.name.clone(),
                artists: meta.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "),
                duration_ms: u32::try_from(meta.duration).unwrap_or(0),
                cover_url: None,
            };
            let album = Some(meta.album.name.as_str());
            println!("has_lyrics bayrağı: {}", meta.has_lyrics);
            let spotify_raw = fetch_spotify(&session, match SpotifyUri::from_uri(&track.uri).unwrap() {
                SpotifyUri::Track { id } => id,
                _ => unreachable!(),
            })
            .await;
            let sp = match &spotify_raw {
                Fetched::Body(b) => parse_spotify(b),
                _ => None,
            };
            let spotify_state = match &spotify_raw {
                Fetched::Body(_) => "yanıt var",
                Fetched::Nothing => "404 / yok",
                Fetched::Failed => "HATA",
            };
            let got = get(&session, &db, &track, album).await;
            println!(
                "{} — {}\n  Spotify doğrudan: {spotify_state}, ayrıştı: {:?}\n  get(): {:?}",
                track.artists,
                track.name,
                sp.as_ref().map(|l| (l.sync, l.lines.len(), l.provider.clone())),
                got.as_ref().map(|l| (l.source, l.sync, l.lines.len(), l.provider.clone())),
            );
            if let Some(l) = &got {
                for line in l.lines.iter().take(3) {
                    println!("    [{:>6} ms] {}", line.start_ms, line.text);
                }
            }
        }
    }

    /// CANLI okuma sınaması (Spotify söz çevirisi): her şarkı için ham söz
    /// yanıtının çeviri alanları (`alternatives` / `translations`) var mı, kaç
    /// dilde, satır sayısı asıl sözle tutuyor mu. Salt okuma; şarkı başına en
    /// çok üç istek (düz uç, web player'ın sorgu dizesiyle, bir de
    /// `app-language: tr` başlığıyla: hangisinde çeviri geliyorsa o). Gerçek
    /// Spotify'a gider; yalnız sahibinin onayıyla, elle koşulur:
    ///
    ///   cargo test --release live_lyrics_translation -- --ignored --nocapture
    ///
    /// Söz metni yazdırılmaz: yalnız sayılar, alan adları ve dil etiketleri.
    /// `OPENIFY_LIVE_TRACKS="spotify:track:…;spotify:track:…"` (en çok 4) ile
    /// şarkılar seçilir; yoksa birkaç İspanyolca / İngilizce / Türkçe olmayan
    /// popüler şarkı. Çeviri hiçbirinde yoksa: uç çeviriyi bu hesapta vermiyor
    /// demektir, düğme hiç görünmez (beklenen güvenli hâl).
    #[tokio::test]
    #[ignore = "canli Spotify: yalniz sahibi onayiyla elle kosulur"]
    async fn live_lyrics_translation() {
        use http::HeaderMap;
        if !crate::auth::has_saved() {
            println!("kayitli giris yok, atlandi");
            return;
        }
        let session = crate::auth::connect(None).await.expect("baglanilamadi");
        let default = "spotify:track:6habFhsOp2NvshLv26DqMb;spotify:track:7qiZfU4dY1lWllzX7mPBI3;spotify:track:2Fxmhks0bxGSBdJ92vM42m";
        let picks = std::env::var("OPENIFY_LIVE_TRACKS").unwrap_or_else(|_| default.into());
        let picks: Vec<&str> = picks.split(';').filter(|s| !s.is_empty()).collect();
        assert!(picks.len() <= 4, "en cok 4 sarki");
        let mut with_translation = 0usize;
        for (i, uri) in picks.iter().enumerate() {
            let Ok(SpotifyUri::Track { id }) = SpotifyUri::from_uri(uri) else { panic!("gecersiz adres: {uri}") };
            let base = format!("/color-lyrics/v2/track/{}", id.to_base62().unwrap());
            let variants: [(&str, String, Option<&str>); 3] = [
                ("duz", base.clone(), None),
                ("web", format!("{base}?format=json&vocalRemoval=false&market=from_token"), None),
                ("web+dil", format!("{base}?format=json&vocalRemoval=false&market=from_token"), Some("tr")),
            ];
            for (name, endpoint, language) in variants {
                tokio::time::sleep(Duration::from_secs(if i == 0 && name == "duz" { 0 } else { 3 })).await;
                let mut headers = HeaderMap::new();
                if let Some(l) = language {
                    headers.insert("app-language", l.parse().unwrap());
                }
                let reply = session
                    .spclient()
                    .request_as_json(&http::Method::GET, &endpoint, Some(headers), None)
                    .await;
                let body = match reply {
                    Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                    Err(e) => {
                        println!("sarki {} [{name}]: istek basarisiz ({:?})", i + 1, e.kind);
                        continue;
                    }
                };
                let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let lyrics = value.get("lyrics");
                let keys: Vec<&str> = lyrics.and_then(Value::as_object).map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
                let alt = lyrics.and_then(|l| l.get("alternatives").or_else(|| l.get("translations"))).and_then(Value::as_array);
                let asil = lyrics.and_then(|l| l.get("lines")).and_then(Value::as_array).map_or(0, Vec::len);
                let langs: Vec<String> = alt
                    .map(|a| {
                        a.iter()
                            .map(|x| {
                                let n = x.get("lines").and_then(Value::as_array).map_or(0, Vec::len);
                                format!("{}:{}satir", x.get("language").and_then(Value::as_str).unwrap_or("?"), n)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let parsed = parse_spotify(&body);
                let kept: Vec<String> = parsed.as_ref().map(|l| l.translations.clone()).unwrap_or_default();
                println!(
                    "sarki {} [{name}]: alan adlari {keys:?}\n    asil satir {asil}, ceviri alani {} ({}), ayristirinca korunan diller {kept:?}",
                    i + 1,
                    if alt.is_some() { "VAR" } else { "yok" },
                    langs.join(", "),
                );
                if !kept.is_empty() {
                    with_translation += 1;
                }
            }
        }
        println!("ozet: {with_translation} yanitta (sarki x varyant) ayristirilmis ceviri var");
    }
}
