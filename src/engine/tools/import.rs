//! Spotify Extended Streaming History içe aktarımı. Dosyalar yalnız okunur;
//! hiçbir Spotify isteği yapılmaz.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use anyhow::{Context, Result};
use rusqlite::params;
use serde_json::Value;

use super::{
    civil,
    plan::primary_artist,
    stats::{self, COUNT_AFTER_MS},
};
use crate::engine::{cache::Db, rules::fold};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: u64,
    pub existing: u64,
    pub skipped: u64,
    pub private_session: bool,
}

#[derive(Debug, Clone)]
struct ImportedPlay {
    played_at: i64,
    uri: String,
    name: String,
    artists: String,
    album: String,
    duration_ms: u32,
    listened_ms: u64,
}

const COLLISION_TOLERANCE_MS: i64 = 90_000;
// 30 minutes covers a single paused listen, but is short enough that a
// separate listen of the same track hours apart is never swallowed.
const MAX_PAUSE_MS: i64 = 30 * 60 * 1_000;

#[derive(Debug)]
struct LocalPlay {
    played_at: i64,
    listened_ms: i64,
    matched: bool,
}

fn overlaps_local_play(
    play: &ImportedPlay,
    local_plays: &mut HashMap<String, Vec<LocalPlay>>,
) -> bool {
    let end = play
        .played_at
        .saturating_add(play.listened_ms.min(i64::MAX as u64) as i64);
    let imported_start = play
        .played_at
        .saturating_sub(MAX_PAUSE_MS)
        .saturating_sub(COLLISION_TOLERANCE_MS);
    let earliest_local_start = end.saturating_sub(MAX_PAUSE_MS);
    let Some(candidates) = local_plays.get_mut(&play.uri) else {
        return false;
    };
    let candidate = candidates
        .iter_mut()
        .filter(|local| {
            if local.matched || local.played_at > end || local.played_at < earliest_local_start {
                return false;
            }
            let local_end = local
                .played_at
                .saturating_add(local.listened_ms)
                .saturating_add(COLLISION_TOLERANCE_MS);
            imported_start <= local_end && local.played_at <= end
        })
        .max_by_key(|local| local.played_at);
    if let Some(local) = candidate {
        local.matched = true;
        true
    } else {
        false
    }
}

fn parse_time(value: &str, short: bool) -> Option<i64> {
    let (date, time) = value.split_once(if short { ' ' } else { 'T' })?;
    let mut d = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let (clock, offset) = if short {
        (time, 0)
    } else if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        (clock, 0)
    } else {
        let split = time
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '+' || *c == '-')?;
        let (clock, zone) = time.split_at(split.0);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let zone = zone.get(1..)?;
        let mut zone = zone.split(':');
        let hours = zone.next()?.parse::<i64>().ok()?;
        let minutes = zone.next()?.parse::<i64>().ok()?;
        (clock, sign * (hours * 60 + minutes))
    };
    let mut clock = clock.split(':');
    let hour = clock.next()?.parse::<u32>().ok()?;
    let minute = clock.next()?.parse::<u32>().ok()?;
    let sec = clock.next().unwrap_or("0");
    let (second, millis) = if let Some((s, fraction)) = sec.split_once('.') {
        let ms = format!("{fraction:0<3}")
            .chars()
            .take(3)
            .collect::<String>()
            .parse()
            .ok()?;
        (s.parse::<u32>().ok()?, ms)
    } else {
        (sec.parse::<u32>().ok()?, 0)
    };
    Some(
        civil::unix_ms(year, month as u32, day as u32, hour, minute, second, millis)
            - offset * 60_000,
    )
}

fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn unsigned(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn row(value: &Value, tracks: &HashMap<(String, String), (String, u32)>) -> Option<ImportedPlay> {
    if value.get("incognito_mode").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let long = value.get("ts").and_then(Value::as_str).is_some();
    let end = if long {
        string(value, "ts")
    } else {
        string(value, "endTime")
    };
    let ended_at = parse_time(&end, !long)?;
    let listened_ms = unsigned(value, if long { "ms_played" } else { "msPlayed" })?;
    let (name, artist, album, uri, duration_ms) = if long {
        let uri = string(value, "spotify_track_uri");
        if !uri.starts_with("spotify:track:") {
            return None;
        }
        (
            string(value, "master_metadata_track_name"),
            string(value, "master_metadata_album_artist_name"),
            string(value, "master_metadata_album_album_name"),
            uri,
            0,
        )
    } else {
        let name = string(value, "trackName");
        let artist = string(value, "artistName");
        let (uri, duration) = tracks.get(&(fold(&artist), fold(&name)))?;
        (name, artist, String::new(), uri.clone(), *duration)
    };
    Some(ImportedPlay {
        played_at: ended_at.saturating_sub(listened_ms.min(i64::MAX as u64) as i64),
        uri,
        name,
        artists: artist,
        album,
        duration_ms,
        listened_ms,
    })
}

fn read_inputs(paths: &[PathBuf]) -> Result<(Vec<Value>, u64)> {
    let mut records = Vec::new();
    let mut skipped_files = 0;
    for path in paths {
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("zip"))
        {
            let (zip_records, zip_skipped) = read_zip(path)?;
            records.extend(zip_records);
            skipped_files += zip_skipped;
        } else {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("{} okunamadı", path.display()))?;
            match serde_json::from_str::<Vec<Value>>(&text) {
                Ok(file_records) => records.extend(file_records),
                Err(_) => skipped_files += 1,
            }
        }
    }
    Ok((records, skipped_files))
}

fn read_zip(path: &std::path::Path) -> Result<(Vec<Value>, u64)> {
    let bytes = std::fs::read(path).with_context(|| format!("{} açılamadı", path.display()))?;
    futures_lite::future::block_on(async {
        let archive = async_zip::base::read::mem::ZipFileReader::new(bytes)
            .await
            .context("ZIP okunamadı")?;
        let mut records = Vec::new();
        let mut skipped_files = 0;
        for index in 0..archive.file().entries().len() {
            let mut reader = archive.reader_with_entry(index).await?;
            let name = reader.entry().filename().as_str().unwrap_or_default();
            let basename = name.rsplit('/').next().unwrap_or_default();
            if !is_history_json(basename) {
                continue;
            }
            let mut content = Vec::new();
            reader.read_to_end_checked(&mut content).await?;
            match serde_json::from_slice::<Vec<Value>>(&content) {
                Ok(file_records) => records.extend(file_records),
                Err(_) => skipped_files += 1,
            }
        }
        Ok((records, skipped_files))
    })
}

fn is_history_json(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".json")
        && !lower.starts_with("streaminghistory_podcast_")
        && (lower.starts_with("streaminghistory") || lower.starts_with("streaming_history_audio"))
}

impl Db {
    pub fn import_spotify_history(
        &self,
        paths: &[PathBuf],
        private_session: bool,
    ) -> Result<ImportSummary> {
        let (source, skipped_files) = read_inputs(paths)?;
        let mut conn = self.w();
        let mut tracks = HashMap::new();
        let mut durations_by_uri = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT uri, name, artists, duration_ms FROM tracks")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u32>(3)?,
                ))
            })?;
            for item in rows {
                let (uri, name, artist, duration) = item?;
                tracks.insert((fold(&artist), fold(&name)), (uri.clone(), duration));
                let primary = primary_artist(&artist);
                if !primary.is_empty() {
                    tracks.insert((fold(primary), fold(&name)), (uri, duration));
                }
            }
            for (uri, duration) in tracks.values() {
                durations_by_uri.insert(uri.clone(), *duration);
            }
        }
        let tx = conn.transaction()?;
        let mut summary = ImportSummary {
            private_session,
            skipped: skipped_files,
            ..Default::default()
        };
        {
            let mut local_plays: HashMap<String, Vec<LocalPlay>> = HashMap::new();
            {
                let mut stmt = tx.prepare_cached(
                    "SELECT uri, played_at, listened_ms FROM play_history WHERE source='local'",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (uri, played_at, listened_ms) = row?;
                    local_plays.entry(uri).or_default().push(LocalPlay {
                        played_at,
                        listened_ms,
                        matched: false,
                    });
                }
            }
            let mut imported_plays = HashSet::new();
            {
                let mut stmt = tx.prepare_cached(
                    "SELECT uri, played_at FROM play_history WHERE source='import'",
                )?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                for row in rows {
                    imported_plays.insert(row?);
                }
            }
            let mut insert = tx.prepare_cached("INSERT OR IGNORE INTO play_history (played_at, uri, name, artists, album, duration_ms, listened_ms, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'import')")?;
            for value in source {
                // A URI-less short-history row can use its cached duration after matching.
                let Some(play) = row(&value, &tracks) else {
                    summary.skipped += 1;
                    continue;
                };
                let duration_ms = durations_by_uri
                    .get(&play.uri)
                    .copied()
                    .unwrap_or(play.duration_ms);
                let threshold = if duration_ms == 0 {
                    COUNT_AFTER_MS
                } else {
                    stats::count_threshold_ms(duration_ms)
                };
                if play.listened_ms < threshold {
                    summary.skipped += 1;
                    continue;
                }
                let key = (play.uri.clone(), play.played_at);
                if imported_plays.contains(&key) || overlaps_local_play(&play, &mut local_plays) {
                    summary.existing += 1;
                    continue;
                }
                let changed = insert.execute(params![
                    play.played_at,
                    play.uri,
                    play.name,
                    play.artists,
                    play.album,
                    duration_ms,
                    play.listened_ms.min(i64::MAX as u64) as i64
                ])?;
                if changed == 1 {
                    imported_plays.insert(key);
                    summary.added += 1;
                } else {
                    summary.existing += 1;
                }
            }
        }
        tx.commit()?;
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_timestamp_and_local_weekday_are_consistent() {
        let end = parse_time("2023-05-01T18:22:31Z", false).unwrap();
        assert_eq!(end - 31_000, civil::unix_ms(2023, 5, 1, 18, 22, 0, 0));
        assert_eq!(
            civil::weekday_of(civil::unix_ms(2023, 5, 1, 0, 0, 0, 0), 180),
            0
        );
    }

    #[test]
    fn long_form_skips_podcasts_private_and_short_plays() {
        let source = r#"[{"ts":"2023-05-01T18:22:31Z","ms_played":31000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:abc"},{"ts":"2023-05-01T18:22:31Z","ms_played":90000,"episode_name":"Podcast","spotify_episode_uri":"spotify:episode:x"},{"ts":"2023-05-01T18:22:31Z","ms_played":90000,"incognito_mode":true,"spotify_track_uri":"spotify:track:hidden"},{"ts":"2023-05-01T18:22:31Z","ms_played":1000,"spotify_track_uri":"spotify:track:short"}]"#;
        let values: Vec<Value> = serde_json::from_str(source).unwrap();
        let tracks = HashMap::new();
        assert!(row(&values[0], &tracks).is_some());
        assert!(row(&values[1], &tracks).is_none());
        assert!(row(&values[2], &tracks).is_none());
        let under_threshold = row(&values[3], &tracks).unwrap();
        assert!(
            under_threshold.listened_ms < stats::count_threshold_ms(under_threshold.duration_ms)
        );
    }

    #[test]
    fn short_form_requires_cached_track_and_uses_play_start_time() {
        let value: Value = serde_json::from_str(r#"{"endTime":"2023-05-01 18:22","artistName":"Artist","trackName":"Song","msPlayed":40000}"#).unwrap();
        let mut tracks = HashMap::new();
        tracks.insert(
            (fold("Artist"), fold("Song")),
            ("spotify:track:abc".into(), 200_000),
        );
        let play = row(&value, &tracks).unwrap();
        assert_eq!(play.played_at, civil::unix_ms(2023, 5, 1, 18, 21, 20, 0));
        assert!(row(&value, &HashMap::new()).is_none());
    }

    fn temp_json(tag: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("openify-history-{tag}-{}.json", std::process::id()));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn importing_the_same_file_twice_counts_duplicates_without_double_inserting() {
        let db = Db::open_in_memory().unwrap();
        let path = temp_json(
            "repeat",
            r#"[{"ts":"2023-05-01T18:22:31Z","ms_played":31000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:abc"}]"#,
        );
        let first = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        let second = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        assert_eq!((first.added, first.existing, first.skipped), (1, 0, 0));
        assert_eq!((second.added, second.existing, second.skipped), (0, 1, 0));
        assert_eq!(db.plays_since(0).unwrap().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn local_play_within_ninety_seconds_is_not_imported() {
        let db = Db::open_in_memory().unwrap();
        let start = civil::unix_ms(2023, 5, 1, 18, 21, 0, 0);
        db.record_play(&stats::PlayRecord {
            uri: "spotify:track:abc".into(),
            name: "Song".into(),
            artists: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            listened_ms: 31_000,
            played_at: start,
        })
        .unwrap();
        let path = temp_json(
            "local-collision",
            r#"[{"ts":"2023-05-01T18:22:31Z","ms_played":31000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:abc"}]"#,
        );
        let summary = db
            .import_spotify_history(std::slice::from_ref(&path), true)
            .unwrap();
        assert_eq!(
            (
                summary.added,
                summary.existing,
                summary.skipped,
                summary.private_session
            ),
            (0, 1, 0, true)
        );
        assert_eq!(db.plays_since(0).unwrap().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn paused_local_play_overlaps_exported_play_interval() {
        let db = Db::open_in_memory().unwrap();
        let actual_start = civil::unix_ms(2023, 5, 1, 18, 21, 0, 0);
        // Locally this row is stamped at the 31 s counting threshold. A five
        // minute pause separates that stamp from Spotify's resumed stream.
        db.record_play(&stats::PlayRecord {
            uri: "spotify:track:pause".into(),
            name: "Song".into(),
            artists: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            listened_ms: 90_000,
            played_at: actual_start + 31_000,
        })
        .unwrap();
        let path = temp_json(
            "paused-local-collision",
            r#"[{"ts":"2023-05-01T18:27:30Z","ms_played":90000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:pause"}]"#,
        );
        let summary = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        assert_eq!((summary.added, summary.existing), (0, 1));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn local_play_two_hours_earlier_does_not_swallow_exported_play() {
        let db = Db::open_in_memory().unwrap();
        db.record_play(&stats::PlayRecord {
            uri: "spotify:track:pause".into(),
            name: "Song".into(),
            artists: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            listened_ms: 90_000,
            played_at: civil::unix_ms(2023, 5, 1, 16, 27, 30, 0),
        })
        .unwrap();
        let path = temp_json(
            "two-hour-gap",
            r#"[{"ts":"2023-05-01T18:27:30Z","ms_played":90000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:pause"}]"#,
        );
        let summary = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        assert_eq!(summary.added, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn consecutive_same_track_plays_consume_distinct_local_rows() {
        let db = Db::open_in_memory().unwrap();
        let start = civil::unix_ms(2023, 5, 1, 18, 0, 0, 0);
        for played_at in [start + 31_000, start + 231_000] {
            db.record_play(&stats::PlayRecord {
                uri: "spotify:track:repeat".into(),
                name: "Song".into(),
                artists: "Artist".into(),
                album: "Album".into(),
                duration_ms: 200_000,
                listened_ms: 120_000,
                played_at,
            })
            .unwrap();
        }
        let path = temp_json(
            "repeat-local-collision",
            r#"[{"ts":"2023-05-01T18:02:00Z","ms_played":120000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:repeat"},{"ts":"2023-05-01T18:05:20Z","ms_played":120000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:repeat"}]"#,
        );
        let summary = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        assert_eq!((summary.added, summary.existing), (0, 2));
        assert_eq!(db.plays_since(0).unwrap().len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unrelated_exported_play_is_added() {
        let db = Db::open_in_memory().unwrap();
        db.record_play(&stats::PlayRecord {
            uri: "spotify:track:local".into(),
            name: "Local".into(),
            artists: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            listened_ms: 120_000,
            played_at: civil::unix_ms(2023, 5, 1, 18, 0, 0, 0),
        })
        .unwrap();
        let path = temp_json(
            "unrelated-play",
            r#"[{"ts":"2023-05-01T18:02:00Z","ms_played":120000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:other"}]"#,
        );
        let summary = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        assert_eq!((summary.added, summary.existing), (1, 0));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn imports_150000_rows_with_years_and_many_uris_in_under_five_seconds() {
        use std::time::Instant;

        let db = Db::open_in_memory().unwrap();
        let mut json = String::with_capacity(45_000_000);
        json.push('[');
        for i in 0..150_000u32 {
            if i > 0 {
                json.push(',');
            }
            let year = 2017 + (i % 9);
            let month = (i / 9) % 12 + 1;
            let day = (i / 108) % 28 + 1;
            let uri_index = i % 15_000;
            if i % 100 == 0 {
                let end = civil::unix_ms(year as i64, month, day, 12, 0, i % 60, 0);
                db.record_play(&stats::PlayRecord {
                    uri: format!("spotify:track:{uri_index}"),
                    name: format!("Song {uri_index}"),
                    artists: "Artist".into(),
                    album: "Album".into(),
                    duration_ms: 0,
                    listened_ms: 120_000,
                    played_at: end - 120_000,
                })
                .unwrap();
            }
            json.push_str(&format!(
                r#"{{"ts":"{year:04}-{month:02}-{day:02}T12:00:{:02}Z","ms_played":120000,"master_metadata_track_name":"Song {uri_index}","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:{uri_index}"}}"#,
                i % 60
            ));
        }
        json.push(']');
        let path = temp_json("150k", &json);
        drop(json);
        let started = Instant::now();
        let summary = db
            .import_spotify_history(std::slice::from_ref(&path), false)
            .unwrap();
        let elapsed = started.elapsed();
        eprintln!("150,000-row import elapsed: {elapsed:.3?}");
        assert_eq!((summary.added, summary.existing), (148_500, 1_500));
        assert!(elapsed.as_secs_f64() < 5.0, "import took {elapsed:.3?}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn zip_reads_short_history_and_ignores_video_history() {
        let path = std::env::temp_dir().join(format!("openify-history-{}.zip", std::process::id()));
        let short_name = b"Spotify/StreamingHistory0.json";
        let video_name = b"Spotify/Streaming_History_Video_1.json";
        let podcast_name = b"Spotify/StreamingHistory_podcast_1.json";
        let malformed_name = b"Spotify/StreamingHistory1.json";
        let short_body = br#"[{"endTime":"2023-05-01 18:22","msPlayed":31000,"trackName":"Song"}]"#;
        let video_body = br#"[{"ts":"2023-05-01T18:22:31Z","ms_played":31000,"spotify_track_uri":"spotify:track:video"}]"#;
        let podcast_body = br#"[{"episodeName":"Podcast"}]"#;
        let malformed_body = b"not JSON";
        std::fs::write(
            &path,
            stored_zip(&[
                (short_name, short_body),
                (video_name, video_body),
                (podcast_name, podcast_body),
                (malformed_name, malformed_body),
            ]),
        )
        .unwrap();
        let (values, skipped) = read_inputs(std::slice::from_ref(&path)).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["trackName"], "Song");
        assert_eq!(skipped, 1);
        let _ = std::fs::remove_file(path);
    }

    fn stored_zip(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central_entries = Vec::new();
        for (name, body) in entries {
            let crc = crc32(body);
            let offset = bytes.len() as u32;
            bytes.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]);
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(body);
            central_entries.push((name, body, crc, offset));
        }
        let central_offset = bytes.len() as u32;
        for (name, body, crc, offset) in central_entries {
            bytes.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]);
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&[0; 12]);
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(name);
        }
        let central_size = bytes.len() as u32 - central_offset;
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
