//! Yerel dinleme istatistiği.
//!
//! Openify'ın KENDİ çaldıklarından tutulan çalma geçmişi (library.db
//! `play_history`, şema sürümü 7). Yalnız bu bilgisayarda durur: Spotify'a
//! hiçbir şey gönderilmez, Spotify'ın kendi geçmişi okunmaz. Gizli oturumda
//! (`private_session`) hiçbir şey yazılmaz.
//!
//! Bir çalma, şarkı DİNLENEREK en az 30 sn ya da süresinin yarısı geçince
//! sayılır (hangisi önceyse; 40 sn'lik bir şarkıda 20 sn). Dinlenen süre duvar
//! saatiyle `Çalıyor` ile `Durdu / Yükleniyor / Bitti / Şarkı değişti`
//! arasında toplanır: ileri sarmak ya da duraklatmak sayacı ilerletmez. Aynı
//! şarkının her yeni başlayışı (tekrar dahil) ayrı çalma.
//!
//! `PlayCounter` saf bir durum makinesi: olayları ve saati dışarıdan alır,
//! yazılacak kayıtları döner (arayüz motora yollar, motor tabloya yazar).
//! Eşik geçilince `Count` (satır eklenir), şarkı bırakılınca `Extend`
//! (toplam dinleme süresine kalanı ekler: "toplam dinleme süresi" yalnız 30'ar
//! sn'lik eşiklerden ibaret kalmasın).

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Result;

use super::{civil, plan::primary_artist};
use crate::engine::{
    cache::Db,
    library::{SmartList, TrackRow},
    rules::fold,
};

// ── Gizli oturum ────────────────────────────────────────────────────────

static PRIVATE_SESSION: AtomicBool = AtomicBool::new(false);

/// Gizli oturumu aç / kapat. (Gizli oturum özelliği henüz yok; geldiğinde bunu
/// çağırır, geçmiş yazımı kendiliğinden durur.)
#[allow(dead_code)] // gizli oturum özelliği gelince çağrılır (şimdilik yalnız testler)
pub fn set_private_session(on: bool) {
    PRIVATE_SESSION.store(on, Ordering::SeqCst);
}

pub fn private_session() -> bool {
    PRIVATE_SESSION.load(Ordering::SeqCst)
}

/// Gizli oturum bayrağı süreç geneli: bayrağı çeviren ya da ona bakan testler
/// sırayla çalışsın.
#[cfg(test)]
pub(crate) static FLAG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ── Çalma sayma ─────────────────────────────────────────────────────────

/// Çalma sayılması için dinlenmesi gereken en uzun süre.
pub const COUNT_AFTER_MS: u64 = 30_000;

/// Bu süredeki şarkı kaç ms dinlenince sayılır: 30 sn ya da yarısı (hangisi
/// önceyse). Süresi bilinmeyen şarkı 30 sn.
pub fn count_threshold_ms(duration_ms: u32) -> u64 {
    if duration_ms == 0 {
        COUNT_AFTER_MS
    } else {
        COUNT_AFTER_MS.min(u64::from(duration_ms) / 2).max(1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackMeta {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u32,
}

/// Tabloya giren bir çalma.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayRecord {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u32,
    /// O çalmada dinlenen süre (ms).
    pub listened_ms: u64,
    /// Çalmanın sayıldığı an, unix ms.
    pub played_at: i64,
}

/// Akıllı listede gösterilen şarkı ve yerel çalma sayısı.
#[derive(Debug, Clone, PartialEq)]
pub struct SmartTrack {
    pub track: TrackRow,
    pub plays: u64,
}

/// Sayacın yazılmasını istediği şey.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayLog {
    /// Yeni çalma (eşik geçildi).
    Count(PlayRecord),
    /// Sayılmış çalmanın dinlenen süresine `add_ms` ekle (`uri` + `played_at` satırı).
    Extend {
        uri: String,
        played_at: i64,
        add_ms: u64,
    },
}

struct Current {
    meta: TrackMeta,
    /// Şimdiye dek dinlenen (ms).
    listened: u64,
    /// Çalıyorsa: dinlenen sürenin en son toplandığı an (monoton ms).
    since: Option<u64>,
    /// Sayıldıysa: kaydın `played_at`ı.
    counted: Option<i64>,
    /// Tabloya yazılmış dinleme süresi.
    flushed: u64,
}

#[derive(Default)]
pub struct PlayCounter {
    cur: Option<Current>,
}

impl PlayCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Yeni şarkı başladı (henüz çalmıyor olabilir): öncekini kapatır.
    pub fn track_changed(&mut self, meta: TrackMeta, mono_ms: u64, wall_ms: i64) -> Vec<PlayLog> {
        let out = self.stopped(mono_ms, wall_ms);
        self.cur = Some(Current {
            meta,
            listened: 0,
            since: None,
            counted: None,
            flushed: 0,
        });
        out
    }

    /// Çalmaya başladı / sürdü (sarma, tamponlamadan çıkış dahil).
    pub fn playing(&mut self, mono_ms: u64, wall_ms: i64) -> Vec<PlayLog> {
        let Some(cur) = self.cur.as_mut() else {
            return Vec::new();
        };
        accumulate(cur, mono_ms);
        cur.since = Some(mono_ms);
        check(cur, wall_ms)
    }

    /// Durdu, yükleniyor, bitti: dinlenen süre toplanır, kalan yazılır.
    pub fn stopped(&mut self, mono_ms: u64, wall_ms: i64) -> Vec<PlayLog> {
        let Some(cur) = self.cur.as_mut() else {
            return Vec::new();
        };
        accumulate(cur, mono_ms);
        cur.since = None;
        let mut out = check(cur, wall_ms);
        out.extend(flush(cur));
        out
    }

    /// Zaman geçti (eşik zamanlayıcısı): eşik geçildiyse `Count`.
    pub fn tick(&mut self, mono_ms: u64, wall_ms: i64) -> Vec<PlayLog> {
        let Some(cur) = self.cur.as_mut() else {
            return Vec::new();
        };
        if cur.since.is_some() {
            accumulate(cur, mono_ms);
            cur.since = Some(mono_ms);
        }
        check(cur, wall_ms)
    }

    /// Çalıyor ve henüz sayılmadıysa: eşiğe kaç ms var (zamanlayıcı için).
    pub fn deadline_ms(&self, mono_ms: u64) -> Option<u64> {
        let cur = self.cur.as_ref()?;
        let since = cur.since?;
        if cur.counted.is_some() {
            return None;
        }
        let listened = cur.listened + mono_ms.saturating_sub(since);
        Some(count_threshold_ms(cur.meta.duration_ms).saturating_sub(listened))
    }
}

fn accumulate(cur: &mut Current, mono_ms: u64) {
    if let Some(since) = cur.since {
        cur.listened += mono_ms.saturating_sub(since);
        cur.since = Some(mono_ms);
    }
}

fn check(cur: &mut Current, wall_ms: i64) -> Vec<PlayLog> {
    if cur.counted.is_some() || cur.listened < count_threshold_ms(cur.meta.duration_ms) {
        return Vec::new();
    }
    // Eşik geçildiği an (olay biraz geç gelmiş olabilir).
    let over = (cur.listened - count_threshold_ms(cur.meta.duration_ms)) as i64;
    let at = wall_ms - over;
    cur.counted = Some(at);
    cur.flushed = cur.listened;
    vec![PlayLog::Count(PlayRecord {
        uri: cur.meta.uri.clone(),
        name: cur.meta.name.clone(),
        artists: cur.meta.artists.clone(),
        album: cur.meta.album.clone(),
        duration_ms: cur.meta.duration_ms,
        listened_ms: cur.listened,
        played_at: at,
    })]
}

fn flush(cur: &mut Current) -> Option<PlayLog> {
    let at = cur.counted?;
    if cur.listened <= cur.flushed {
        return None;
    }
    let add = cur.listened - cur.flushed;
    cur.flushed = cur.listened;
    Some(PlayLog::Extend {
        uri: cur.meta.uri.clone(),
        played_at: at,
        add_ms: add,
    })
}

// ── Tablo ───────────────────────────────────────────────────────────────

impl Db {
    pub fn record_play(&self, rec: &PlayRecord) -> Result<()> {
        self.w().execute(
            "INSERT INTO play_history (played_at, uri, name, artists, album, duration_ms, listened_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                rec.played_at,
                rec.uri,
                rec.name,
                rec.artists,
                rec.album,
                rec.duration_ms,
                rec.listened_ms as i64
            ],
        )?;
        Ok(())
    }

    pub fn extend_play(&self, uri: &str, played_at: i64, add_ms: u64) -> Result<()> {
        self.w().execute(
            "UPDATE play_history SET listened_ms = listened_ms + ?3 WHERE uri = ?1 AND played_at = ?2",
            rusqlite::params![uri, played_at, add_ms as i64],
        )?;
        Ok(())
    }

    /// `since_ms`ten (dahil) sonraki çalmalar, eskiden yeniye.
    pub fn plays_since(&self, since_ms: i64) -> Result<Vec<PlayRecord>> {
        let conn = self.r();
        let mut stmt = conn.prepare_cached(
            "SELECT played_at, uri, name, artists, album, duration_ms, listened_ms
             FROM play_history WHERE played_at >= ?1 ORDER BY played_at, id",
        )?;
        let rows = stmt
            .query_map([since_ms], |r| {
                Ok(PlayRecord {
                    played_at: r.get(0)?,
                    uri: r.get(1)?,
                    name: r.get(2)?,
                    artists: r.get(3)?,
                    album: r.get(4)?,
                    duration_ms: r.get(5)?,
                    listened_ms: r.get::<_, i64>(6)?.max(0) as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Akıllı listeyi yalnız yerel geçmiş ve önbellekten hesaplar.
    pub fn smart_tracks(&self, list: SmartList, now_ms: i64) -> Result<Vec<SmartTrack>> {
        const DAY: i64 = 86_400_000;
        let conn = self.r();
        let mut stmt = match list {
            SmartList::Top30 => conn.prepare(
                "WITH counts AS (
                    SELECT uri, COUNT(*) AS plays, MAX(played_at) AS last_at
                    FROM play_history WHERE played_at >= ?1 GROUP BY uri
                 ), latest AS (
                    SELECT uri, name, artists, album, duration_ms,
                           ROW_NUMBER() OVER (PARTITION BY uri ORDER BY played_at DESC, id DESC) AS rank
                    FROM play_history WHERE played_at >= ?1
                 )
                 SELECT c.uri, COALESCE(NULLIF(t.name,''), l.name),
                        COALESCE(NULLIF(t.artists,''), l.artists),
                        COALESCE(NULLIF(t.album,''), l.album),
                        COALESCE(t.duration_ms, l.duration_ms), COALESCE(t.image,''), c.plays
                 FROM counts c JOIN latest l ON l.uri = c.uri AND l.rank = 1
                 LEFT JOIN tracks t ON t.uri = c.uri
                 ORDER BY c.plays DESC, c.last_at DESC, c.uri LIMIT 50",
            )?,
            SmartList::Unplayed => conn.prepare(
                "SELECT ct.track_uri, COALESCE(NULLIF(t.name,''), ct.track_uri),
                        COALESCE(t.artists,''), COALESCE(t.album,''), COALESCE(t.duration_ms,0),
                        COALESCE(t.image,''), 0
                 FROM collection_tracks ct
                 LEFT JOIN tracks t ON t.uri = ct.track_uri
                 WHERE ct.collection = 'liked'
                   AND NOT EXISTS (SELECT 1 FROM play_history h WHERE h.uri = ct.track_uri)
                 ORDER BY ct.added_at DESC, ct.position LIMIT -1",
            )?,
            SmartList::Forgotten => conn.prepare(
                "WITH counts AS (
                    SELECT uri, COUNT(*) AS plays, MAX(played_at) AS last_at
                    FROM play_history GROUP BY uri
                 ), latest AS (
                    SELECT uri, name, artists, album, duration_ms,
                           ROW_NUMBER() OVER (PARTITION BY uri ORDER BY played_at DESC, id DESC) AS rank
                    FROM play_history
                 )
                 SELECT c.uri, COALESCE(NULLIF(t.name,''), l.name),
                        COALESCE(NULLIF(t.artists,''), l.artists),
                        COALESCE(NULLIF(t.album,''), l.album),
                        COALESCE(t.duration_ms, l.duration_ms), COALESCE(t.image,''), c.plays
                 FROM counts c JOIN latest l ON l.uri = c.uri AND l.rank = 1
                 LEFT JOIN tracks t ON t.uri = c.uri
                 WHERE c.plays >= 3 AND c.last_at < ?1
                 ORDER BY c.plays DESC, c.last_at ASC, c.uri LIMIT 50",
            )?,
        };
        let rows = stmt
            .query_map(
                // Beğenip hiç çalmadıkların zamana bakmaz: yeni beğenilen şarkı da
                // hemen girer. Öbürleri pencere başlangıcını alır.
                rusqlite::params_from_iter(match list {
                    SmartList::Top30 => vec![now_ms - 30 * DAY],
                    SmartList::Forgotten => vec![now_ms - 60 * DAY],
                    SmartList::Unplayed => Vec::new(),
                }),
                |r| {
                    Ok(SmartTrack {
                        track: TrackRow {
                            uri: r.get(0)?,
                            name: r.get(1)?,
                            artists: r.get(2)?,
                            album: r.get(3)?,
                            duration_ms: r.get(4)?,
                            image: r.get(5)?,
                            plays: Some(r.get::<_, i64>(6)?.max(0) as u64),
                        },
                        plays: r.get::<_, i64>(6)?.max(0) as u64,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

// ── Toplama ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StatsRange {
    /// Son 4 hafta (28 gün).
    #[default]
    Weeks4,
    /// Son 6 ay (182 gün).
    Months6,
    All,
}

impl StatsRange {
    pub const ALL: [StatsRange; 3] = [StatsRange::Weeks4, StatsRange::Months6, StatsRange::All];

    /// Aralığın başlangıcı (unix ms); `All` için 0.
    pub fn since_ms(self, now_ms: i64) -> i64 {
        const DAY: i64 = 86_400_000;
        match self {
            Self::Weeks4 => now_ms - 28 * DAY,
            Self::Months6 => now_ms - 182 * DAY,
            Self::All => 0,
        }
    }
}

/// Listelerde en çok bu kadar satır hesaplanır (arayüz azını gösterir).
pub const TOP_N: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct TopTrack {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub plays: u32,
    pub listened_ms: u64,
    pub image: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopArtist {
    pub name: String,
    pub plays: u32,
    pub listened_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopAlbum {
    pub album: String,
    pub artist: String,
    pub plays: u32,
    pub listened_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatsReport {
    pub range: StatsRange,
    /// Sayılan çalma sayısı.
    pub plays: u64,
    pub unique_tracks: u64,
    pub unique_artists: u64,
    /// Toplam dinleme süresi (ms).
    pub listened_ms: u64,
    pub tracks: Vec<TopTrack>,
    pub artists: Vec<TopArtist>,
    pub albums: Vec<TopAlbum>,
    /// Yerel saate göre saat başına dinlenen süre (ms), 0..=23.
    pub hours: [u64; 24],
    /// Yerel güne göre dinlenen süre (Pazartesi 0, Pazar 6).
    pub weekdays: [u64; 7],
}

fn rank_key(
    listened: u64,
    plays: u32,
    name: &str,
) -> (std::cmp::Reverse<u64>, std::cmp::Reverse<u32>, String) {
    (
        std::cmp::Reverse(listened),
        std::cmp::Reverse(plays),
        fold(name),
    )
}

/// `records` (zaten aralıkta) üzerinden raporu çıkarır. Sanatçı: şarkının ana
/// (ilk) sanatçısı. Sıra: çalma sayısı, eşitlikte dinlenen süre, sonra ad.
pub fn aggregate(range: StatsRange, records: &[PlayRecord], offset_minutes: i64) -> StatsReport {
    let mut report = StatsReport {
        range,
        ..Default::default()
    };
    let mut tracks: HashMap<&str, TopTrack> = HashMap::new();
    let mut artists: HashMap<String, TopArtist> = HashMap::new();
    let mut albums: HashMap<(String, String), TopAlbum> = HashMap::new();
    for r in records {
        report.plays += 1;
        report.listened_ms += r.listened_ms;
        report.hours[civil::hour_of(r.played_at, offset_minutes).min(23)] += r.listened_ms;
        report.weekdays[civil::weekday_of(r.played_at, offset_minutes).min(6)] += r.listened_ms;
        let t = tracks.entry(r.uri.as_str()).or_insert_with(|| TopTrack {
            uri: r.uri.clone(),
            name: r.name.clone(),
            artists: r.artists.clone(),
            plays: 0,
            listened_ms: 0,
            image: String::new(),
        });
        t.plays += 1;
        t.listened_ms += r.listened_ms;
        // Kayıt eskiden yeniye: en yeni ad / sanatçı yazımı kalır.
        t.name.clone_from(&r.name);
        t.artists.clone_from(&r.artists);
        let artist = primary_artist(&r.artists);
        if !artist.is_empty() {
            let a = artists.entry(fold(artist)).or_insert_with(|| TopArtist {
                name: artist.to_string(),
                plays: 0,
                listened_ms: 0,
            });
            a.plays += 1;
            a.listened_ms += r.listened_ms;
        }
        if !r.album.trim().is_empty() {
            let key = (fold(&r.album), fold(artist));
            let a = albums.entry(key).or_insert_with(|| TopAlbum {
                album: r.album.clone(),
                artist: artist.to_string(),
                plays: 0,
                listened_ms: 0,
            });
            a.plays += 1;
            a.listened_ms += r.listened_ms;
        }
    }
    report.unique_tracks = tracks.len() as u64;
    report.unique_artists = artists.len() as u64;
    report.tracks = tracks.into_values().collect();
    report
        .tracks
        .sort_by_key(|t| rank_key(t.listened_ms, t.plays, &t.name));
    report.tracks.truncate(TOP_N);
    report.artists = artists.into_values().collect();
    report
        .artists
        .sort_by_key(|a| rank_key(a.listened_ms, a.plays, &a.name));
    report.artists.truncate(TOP_N);
    report.albums = albums.into_values().collect();
    report
        .albums
        .sort_by_key(|a| rank_key(a.listened_ms, a.plays, &a.album));
    report.albums.truncate(TOP_N);
    report
}

/// Veritabanından, verilen aralık ve anda rapor.
pub fn report(db: &Db, range: StatsRange, now_ms: i64, offset_minutes: i64) -> Result<StatsReport> {
    let rows = db.plays_since(range.since_ms(now_ms))?;
    let mut report = aggregate(range, &rows, offset_minutes);
    let conn = db.r();
    let mut stmt = conn.prepare_cached("SELECT image FROM tracks WHERE uri=?1")?;
    for track in &mut report.tracks {
        track.image = stmt
            .query_row([&track.uri], |r| r.get(0))
            .unwrap_or_default();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1000;

    fn meta(n: u32, ms: u32) -> TrackMeta {
        TrackMeta {
            uri: format!("spotify:track:{n:022}"),
            name: format!("Şarkı {n}"),
            artists: "Sanatçı".into(),
            album: "Albüm".into(),
            duration_ms: ms,
        }
    }

    fn counts(logs: &[PlayLog]) -> usize {
        logs.iter()
            .filter(|l| matches!(l, PlayLog::Count(_)))
            .count()
    }

    #[test]
    fn threshold_is_thirty_seconds_or_half_the_track() {
        assert_eq!(count_threshold_ms(200_000), 30_000);
        assert_eq!(count_threshold_ms(60_000), 30_000);
        assert_eq!(count_threshold_ms(40_000), 20_000);
        assert_eq!(count_threshold_ms(3_000), 1_500);
        assert_eq!(count_threshold_ms(0), 30_000);
        assert_eq!(count_threshold_ms(1), 1);
    }

    #[test]
    fn a_play_counts_at_thirty_seconds_and_the_rest_is_added_when_it_stops() {
        let mut c = PlayCounter::new();
        let wall = 1_700_000_000_000;
        assert!(c.track_changed(meta(1, 200_000), 0, wall).is_empty());
        assert!(c.playing(0, wall).is_empty());
        assert_eq!(c.deadline_ms(10 * S), Some(20 * S));
        assert!(
            c.tick(29 * S, wall + 29_000).is_empty(),
            "29 sn: henüz sayılmaz"
        );
        let logs = c.tick(31 * S, wall + 31_000);
        assert_eq!(counts(&logs), 1);
        let PlayLog::Count(rec) = &logs[0] else {
            panic!()
        };
        assert_eq!(
            (rec.listened_ms, rec.played_at),
            (31_000, wall + 30_000),
            "an eşik geçildiği an"
        );
        assert_eq!(
            c.deadline_ms(32 * S),
            None,
            "sayıldıktan sonra zamanlayıcı yok"
        );
        // 100. saniyede duraklat: kalan 69 sn eklenir (31 → 100).
        let logs = c.stopped(100 * S, wall + 100_000);
        assert_eq!(
            logs,
            [PlayLog::Extend {
                uri: meta(1, 0).uri,
                played_at: wall + 30_000,
                add_ms: 69_000
            }]
        );
        // Bir daha durdurmak ek yazmaz.
        assert!(c.stopped(120 * S, wall + 120_000).is_empty());
    }

    #[test]
    fn under_thirty_seconds_never_counts_and_writes_nothing() {
        let mut c = PlayCounter::new();
        c.track_changed(meta(1, 200_000), 0, 0);
        c.playing(0, 0);
        assert!(c.stopped(29 * S, 29_000).is_empty());
        // Sonra sürse de toplam 30'a varınca sayılır (duraklatmak sayacı sıfırlamaz).
        c.playing(100 * S, 100_000);
        let logs = c.tick(102 * S, 102_000);
        assert_eq!(counts(&logs), 1);
        let logs = c.stopped(102 * S, 102_000);
        assert!(logs.is_empty());
    }

    #[test]
    fn skipping_ahead_and_pausing_do_not_advance_the_counter() {
        let mut c = PlayCounter::new();
        c.track_changed(meta(1, 200_000), 0, 0);
        c.playing(0, 0);
        // Duraklatılmış 10 dk bekleme dinleme sayılmaz.
        c.stopped(5 * S, 5_000);
        assert!(c.tick(600 * S, 600_000).is_empty());
        assert_eq!(c.deadline_ms(600 * S), None, "durmuşken zamanlayıcı yok");
        c.playing(600 * S, 600_000);
        assert_eq!(c.deadline_ms(600 * S), Some(25 * S));
        // Sarma "Çalıyor" olayını yeniden getirir: çift sayım yok.
        c.playing(610 * S, 610_000);
        assert_eq!(c.deadline_ms(610 * S), Some(15 * S));
    }

    #[test]
    fn short_tracks_count_at_half() {
        let mut c = PlayCounter::new();
        c.track_changed(meta(1, 40_000), 0, 0);
        c.playing(0, 0);
        assert!(c.tick(19 * S, 19_000).is_empty());
        assert_eq!(counts(&c.tick(21 * S, 21_000)), 1);
    }

    #[test]
    fn changing_track_closes_the_previous_one_and_a_replay_counts_again() {
        let mut c = PlayCounter::new();
        c.track_changed(meta(1, 200_000), 0, 0);
        c.playing(0, 0);
        c.tick(31 * S, 31_000);
        // Şarkı 45. saniyede değişti: önceki kapanır (14 sn ek), yenisi sıfırdan.
        let logs = c.track_changed(meta(1, 200_000), 45 * S, 45_000);
        assert_eq!(
            logs,
            [PlayLog::Extend {
                uri: meta(1, 0).uri,
                played_at: 30_000,
                add_ms: 14_000
            }]
        );
        c.playing(45 * S, 45_000);
        let logs = c.tick(76 * S, 76_000);
        assert_eq!(
            counts(&logs),
            1,
            "aynı şarkının yeniden başlaması ayrı çalma"
        );
    }

    #[test]
    fn a_late_event_still_counts_once() {
        let mut c = PlayCounter::new();
        c.track_changed(meta(1, 200_000), 0, 0);
        c.playing(0, 0);
        // Zamanlayıcı kaçtı; ilk olay 90 sn sonra (duraklat) geldi: bir Count + bir Extend.
        let logs = c.stopped(90 * S, 90_000);
        assert_eq!(counts(&logs), 1);
        let PlayLog::Count(rec) = &logs[0] else {
            panic!()
        };
        assert_eq!((rec.listened_ms, rec.played_at), (90_000, 30_000));
        assert_eq!(logs.len(), 1, "Count zaten 90 sn'yi yazdı, ek yok");
    }

    #[test]
    fn events_without_a_track_are_ignored() {
        let mut c = PlayCounter::new();
        assert!(
            c.playing(0, 0).is_empty() && c.tick(1, 1).is_empty() && c.stopped(2, 2).is_empty()
        );
        assert_eq!(c.deadline_ms(3), None);
    }

    #[test]
    fn private_session_flag_round_trips() {
        let _guard = FLAG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!private_session());
        set_private_session(true);
        assert!(private_session());
        set_private_session(false);
        assert!(!private_session());
    }

    // ── Tablo ve toplama ────────────────────────────────────────────────

    fn rec(n: u32, artists: &str, album: &str, listened: u64, at: i64) -> PlayRecord {
        PlayRecord {
            uri: format!("spotify:track:{n:022}"),
            name: format!("Şarkı {n}"),
            artists: artists.into(),
            album: album.into(),
            duration_ms: 200_000,
            listened_ms: listened,
            played_at: at,
        }
    }

    #[test]
    fn smart_top30_counts_only_recent_plays_and_caps_at_fifty() {
        use crate::engine::library::SmartList;
        let db = Db::open_in_memory().unwrap();
        let now = 1_800_000_000_000;
        for (n, count) in [(1, 2), (2, 3), (3, 1)] {
            for i in 0..count {
                db.record_play(&rec(
                    n,
                    "Artist",
                    "Album",
                    40_000,
                    now - i64::from(i) * 1000,
                ))
                .unwrap();
            }
        }
        db.record_play(&rec(1, "Artist", "Album", 40_000, now - 31 * 86_400_000))
            .unwrap();
        db.record_play(&rec(4, "Artist", "Album", 40_000, now - 30 * 86_400_000))
            .unwrap();
        let got = db.smart_tracks(SmartList::Top30, now).unwrap();
        assert_eq!(
            got.iter()
                .map(|x| (x.track.uri.as_str(), x.plays))
                .collect::<Vec<_>>(),
            [
                ("spotify:track:0000000000000000000002", 3),
                ("spotify:track:0000000000000000000001", 2),
                ("spotify:track:0000000000000000000003", 1),
                ("spotify:track:0000000000000000000004", 1),
            ]
        );
    }

    #[test]
    fn smart_unplayed_uses_liked_order_and_excludes_any_history() {
        use crate::engine::{
            TrackRow,
            library::{Entry, SmartList},
        };
        let db = Db::open_in_memory().unwrap();
        let now = 1_800_000_000_000;
        let rows: Vec<_> = (1..=3)
            .map(|n| TrackRow {
                uri: format!("spotify:track:{n:022}"),
                name: format!("Cached {n}"),
                artists: "Artist".into(),
                album: "Album".into(),
                duration_ms: 1000,
                image: String::new(),
                plays: None,
            })
            .collect();
        db.save_tracks(&rows).unwrap();
        db.save_collection_entries(
            &crate::engine::CollectionId::Liked,
            None,
            &[
                Entry {
                    track: rows[0].uri.clone(),
                    added_at: Some(10),
                },
                Entry {
                    track: rows[1].uri.clone(),
                    // Dün beğenildi: yeni beğeniler de hemen listeye girer.
                    added_at: Some(now - 86_400_000),
                },
                Entry {
                    track: rows[2].uri.clone(),
                    added_at: Some(20),
                },
            ],
        )
        .unwrap();
        db.record_play(&rec(1, "Old", "Old", 1000, now - 100 * 86_400_000))
            .unwrap();
        let got = db.smart_tracks(SmartList::Unplayed, now).unwrap();
        assert_eq!(
            got.iter()
                .map(|x| x.track.name.as_str())
                .collect::<Vec<_>>(),
            ["Cached 2", "Cached 3"]
        );
    }

    #[test]
    fn smart_forgotten_requires_three_total_plays_and_sixty_quiet_days() {
        use crate::engine::library::SmartList;
        let db = Db::open_in_memory().unwrap();
        let now = 1_800_000_000_000;
        for i in 0..4 {
            db.record_play(&rec(1, "A", "X", 1000, now - 61 * 86_400_000 - i))
                .unwrap();
        }
        for i in 0..3 {
            db.record_play(&rec(2, "B", "Y", 1000, now - 61 * 86_400_000 - i))
                .unwrap();
        }
        for i in 0..2 {
            db.record_play(&rec(3, "C", "Z", 1000, now - 90 * 86_400_000 - i))
                .unwrap();
        }
        for i in 0..4 {
            db.record_play(&rec(4, "D", "W", 1000, now - 10 * 86_400_000 - i))
                .unwrap();
        }
        for i in 0..3 {
            db.record_play(&rec(5, "E", "V", 1000, now - 60 * 86_400_000 - i))
                .unwrap();
        }
        let got = db.smart_tracks(SmartList::Forgotten, now).unwrap();
        assert_eq!(
            got.iter()
                .map(|x| (x.track.uri.as_str(), x.plays))
                .collect::<Vec<_>>(),
            [
                ("spotify:track:0000000000000000000001", 4),
                ("spotify:track:0000000000000000000002", 3),
            ]
        );
    }

    #[test]
    fn smart_lists_are_empty_without_qualifying_data_and_ranked_lists_stop_at_fifty() {
        use crate::engine::library::SmartList;
        let db = Db::open_in_memory().unwrap();
        let now = 1_800_000_000_000;
        for list in SmartList::ALL {
            assert!(db.smart_tracks(list, now).unwrap().is_empty());
        }
        for n in 0..55 {
            let uri = format!("spotify:track:{n:022}");
            let mut current = rec(n, "Artist", "Album", 1000, now - 86_400_000);
            current.uri = uri;
            db.record_play(&current).unwrap();
            for day in 70..73 {
                let mut old = rec(n + 100, "Artist", "Album", 1000, now - day * 86_400_000);
                old.uri = format!("spotify:track:{:022}", n + 1000);
                db.record_play(&old).unwrap();
            }
        }
        assert_eq!(db.smart_tracks(SmartList::Top30, now).unwrap().len(), 50);
        assert_eq!(
            db.smart_tracks(SmartList::Forgotten, now).unwrap().len(),
            50
        );
    }

    #[test]
    fn smart_lists_prefer_cached_metadata_and_fall_back_to_play_history() {
        use crate::engine::{TrackRow, library::SmartList};
        let db = Db::open_in_memory().unwrap();
        let now = 1_800_000_000_000;
        let history = rec(1, "History artist", "History album", 1000, now - 1000);
        db.record_play(&history).unwrap();
        let cached = TrackRow {
            uri: history.uri.clone(),
            name: "Cached title".into(),
            artists: "Cached artist".into(),
            album: "Cached album".into(),
            duration_ms: 123_000,
            image: "cover-url".into(),
            plays: None,
        };
        db.save_tracks(&[cached]).unwrap();
        let smart = db.smart_tracks(SmartList::Top30, now).unwrap();
        assert_eq!(smart[0].track.name, "Cached title");
        assert_eq!(smart[0].track.artists, "Cached artist");
        assert_eq!(smart[0].track.album, "Cached album");
        assert_eq!(smart[0].track.duration_ms, 123_000);
        assert_eq!(smart[0].track.image, "cover-url");

        let fallback = rec(2, "History artist", "History album", 1000, now - 2000);
        db.record_play(&fallback).unwrap();
        let smart = db.smart_tracks(SmartList::Top30, now).unwrap();
        let row = smart.iter().find(|x| x.track.uri == fallback.uri).unwrap();
        assert_eq!(row.track.name, fallback.name);
        assert_eq!(row.track.artists, fallback.artists);
        assert_eq!(row.track.album, fallback.album);
    }

    #[test]
    fn the_table_round_trips_counts_and_extends() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.plays_since(0).unwrap().is_empty());
        let a = rec(1, "A, B", "X", 31_000, 1_000);
        db.record_play(&a).unwrap();
        db.record_play(&rec(2, "C", "Y", 40_000, 5_000)).unwrap();
        db.extend_play(&a.uri, 1_000, 9_000).unwrap();
        // Olmayan satıra ek zararsız.
        db.extend_play(&a.uri, 77, 1).unwrap();
        let all = db.plays_since(0).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            (all[0].uri.as_str(), all[0].listened_ms),
            (a.uri.as_str(), 40_000)
        );
        assert_eq!(db.plays_since(2_000).unwrap().len(), 1, "aralık başlangıcı");
        assert_eq!(db.plays_since(5_000).unwrap().len(), 1, "başlangıç dahil");
    }

    #[test]
    fn aggregate_ranks_by_listened_time_then_plays_and_sums_local_buckets() {
        // Yerel saat farkı +3: UTC 09:00 → 12:00.
        let nine = 9 * 3_600_000i64;
        let records = vec![
            rec(1, "Mavi Orman", "Uzun Yol", 200_000, nine),
            rec(2, "Mavi Orman, Konuk", "Uzun Yol", 100_000, nine + 1000),
            rec(1, "Mavi Orman", "Uzun Yol", 150_000, nine + 2000),
            rec(3, "Kuzey", "Kuzey Işıkları", 300_000, nine + 3_600_000),
            rec(4, "kuzey", "Kuzey Işıkları", 60_000, 20 * 3_600_000),
        ];
        let r = aggregate(StatsRange::All, &records, 180);
        assert_eq!(r.plays, 5);
        assert_eq!(r.listened_ms, 810_000);
        assert_eq!(r.tracks[0].uri, records[0].uri);
        assert_eq!((r.tracks[0].plays, r.tracks[0].listened_ms), (2, 350_000));
        // Sanatçılar: Mavi Orman 3 çalma; "Kuzey" ve "kuzey" aynı sanatçı (2).
        assert_eq!(
            r.artists
                .iter()
                .map(|a| (a.name.as_str(), a.plays))
                .collect::<Vec<_>>(),
            [("Mavi Orman", 3), ("Kuzey", 2)]
        );
        assert_eq!(r.albums[0].album, "Uzun Yol");
        assert_eq!((r.albums[0].plays, r.albums[1].plays), (3, 2));
        assert_eq!(r.hours[12], 450_000, "saat 12 (UTC 09 + 3)");
        assert_eq!(r.hours[13], 300_000);
        assert_eq!(r.hours[23], 60_000, "UTC 20 + 3 = 23");
        assert_eq!(r.hours.iter().sum::<u64>(), r.listened_ms);
        assert_eq!(
            r.weekdays[3], r.listened_ms,
            "epoch günü UTC ve +3 yerel aynı Perşembe"
        );
        assert_eq!(r.weekdays.iter().sum::<u64>(), r.listened_ms);
        assert_eq!((r.unique_tracks, r.unique_artists), (4, 2));
    }

    #[test]
    fn aggregate_caps_the_lists_and_handles_an_empty_history() {
        let none = aggregate(StatsRange::Weeks4, &[], 0);
        assert_eq!((none.plays, none.listened_ms), (0, 0));
        assert!(none.tracks.is_empty() && none.artists.is_empty() && none.albums.is_empty());
        let many: Vec<PlayRecord> = (0..40)
            .map(|n| rec(n, &format!("S{n}"), &format!("A{n}"), 1000, n as i64))
            .collect();
        let r = aggregate(StatsRange::All, &many, 0);
        assert_eq!(
            (r.tracks.len(), r.artists.len(), r.albums.len()),
            (TOP_N, TOP_N, TOP_N)
        );
    }

    #[test]
    fn ranges_start_where_they_say() {
        let now = 1_800_000_000_000;
        assert_eq!(StatsRange::All.since_ms(now), 0);
        assert_eq!(now - StatsRange::Weeks4.since_ms(now), 28 * 86_400_000);
        assert_eq!(now - StatsRange::Months6.since_ms(now), 182 * 86_400_000);
        // Veritabanından aralık: eski çalma 4 haftalıkta yok, tümünde var.
        let db = Db::open_in_memory().unwrap();
        db.record_play(&rec(1, "A", "X", 1000, now - 40 * 86_400_000))
            .unwrap();
        db.record_play(&rec(2, "B", "Y", 1000, now - 86_400_000))
            .unwrap();
        assert_eq!(report(&db, StatsRange::Weeks4, now, 0).unwrap().plays, 1);
        assert_eq!(report(&db, StatsRange::Months6, now, 0).unwrap().plays, 2);
        assert_eq!(report(&db, StatsRange::All, now, 0).unwrap().plays, 2);
    }

    /// Risk ölçümü: 100 bin çalmalık geçmişin toplanması. Çıta: < 500 ms.
    #[test]
    fn aggregating_a_hundred_thousand_plays_is_fast() {
        let records: Vec<PlayRecord> = (0..100_000u32)
            .map(|i| {
                rec(
                    i % 3000,
                    &format!("Sanatçı {}", i % 400),
                    &format!("Albüm {}", i % 900),
                    100_000,
                    i64::from(i) * 60_000,
                )
            })
            .collect();
        // Üç denemenin en iyisi: makine başka derlemelerle doluyken tek ölçüm sapar.
        let mut ms = f64::MAX;
        let mut r = aggregate(StatsRange::All, &records, 180);
        for _ in 0..3 {
            let t = std::time::Instant::now();
            r = aggregate(StatsRange::All, &records, 180);
            ms = ms.min(t.elapsed().as_secs_f64() * 1000.);
        }
        println!("ölçüm: 100000 çalma toplama {ms:.1} ms");
        assert_eq!(r.plays, 100_000);
        assert!(ms < 500., "{ms} ms");
    }
}
