//! Yerel çalma listesi içerik geçmişi.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;
use rusqlite::{Transaction, params};
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Moved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub uri: String,
    pub from: Option<usize>,
    pub to: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChange {
    pub id: i64,
    pub at: i64,
    pub kind: ChangeKind,
    pub uri: String,
    pub from: Option<usize>,
    pub to: Option<usize>,
    pub added_by: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub baseline_at: i64,
    pub baseline: Vec<String>,
    pub events: Vec<StoredChange>,
}

pub fn state_at(current: &[String], events: &[StoredChange], at: i64) -> Vec<String> {
    let mut state = current.to_vec();
    for event in events.iter().rev().filter(|e| e.at > at) {
        match event.kind {
            ChangeKind::Added => {
                if let Some(i) = find_occurrence(&state, &event.uri, event.to) {
                    state.remove(i);
                }
            }
            ChangeKind::Removed => {
                let i = event.from.unwrap_or(state.len()).min(state.len());
                state.insert(i, event.uri.clone());
            }
            ChangeKind::Moved => {
                if let Some(i) = find_occurrence(&state, &event.uri, event.to) {
                    let item = state.remove(i);
                    let to = event.from.unwrap_or(state.len()).min(state.len());
                    state.insert(to, item);
                }
            }
        }
    }
    state
}

fn find_occurrence(state: &[String], uri: &str, position: Option<usize>) -> Option<usize> {
    if let Some(i) = position.filter(|i| state.get(*i).is_some_and(|item| item == uri)) {
        return Some(i);
    }
    state.iter().position(|item| item == uri)
}

pub const MAX_EVENTS_PER_LIST: i64 = 500;
pub const MAX_AGE_MS: i64 = 365 * 24 * 60 * 60 * 1000;
pub const MAX_BYTES: i64 = 20 * 1024 * 1024;

pub fn migration(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS playlist_history_baselines (
            collection TEXT PRIMARY KEY, observed_at INTEGER NOT NULL, entries TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS playlist_history_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT, collection TEXT NOT NULL,
            at INTEGER NOT NULL, kind TEXT NOT NULL, track_uri TEXT NOT NULL,
            from_position INTEGER, to_position INTEGER, added_by TEXT,
            source TEXT NOT NULL DEFAULT 'spotify'
         );
         CREATE INDEX IF NOT EXISTS playlist_history_collection_at
            ON playlist_history_events (collection, at DESC);
         CREATE INDEX IF NOT EXISTS playlist_history_at
            ON playlist_history_events (at);",
    )
}

/// Persist only the first-seen baseline and later content deltas.
pub fn record(
    tx: &Transaction,
    collection: &str,
    before: Option<&[String]>,
    after: &[String],
    at: i64,
    source: &str,
) -> Result<()> {
    if !collection.starts_with("spotify:playlist:") {
        return Ok(());
    }
    let had_baseline: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM playlist_history_baselines WHERE collection = ?1)",
        [collection],
        |row| row.get(0),
    )?;
    if !had_baseline {
        let entries = serde_json::to_string(after)?;
        tx.execute(
            "INSERT INTO playlist_history_baselines (collection, observed_at, entries) VALUES (?1, ?2, ?3)",
            params![collection, at, entries],
        )?;
        prune(tx, at)?;
        return Ok(());
    }
    let Some(before) = before else {
        return Ok(());
    };
    if before == after {
        return Ok(());
    }
    for change in diff(before, after) {
        let kind = match change.kind {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Moved => "moved",
        };
        tx.execute(
            "INSERT INTO playlist_history_events (collection, at, kind, track_uri, from_position, to_position, added_by, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
            params![collection, at, kind, change.uri, change.from.map(|x| x as i64), change.to.map(|x| x as i64), source],
        )?;
    }
    prune(tx, at)?;
    Ok(())
}

pub fn prune(tx: &Transaction, now: i64) -> Result<()> {
    let cutoff = now - MAX_AGE_MS;
    tx.execute(
        "DELETE FROM playlist_history_events WHERE at < ?1",
        [cutoff],
    )?;
    let stale: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT collection FROM playlist_history_baselines WHERE observed_at < ?1
             AND EXISTS (SELECT 1 FROM playlist_history_events e WHERE e.collection = playlist_history_baselines.collection)",
        )?;
        stmt.query_map([cutoff], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for collection in stale {
        let entries: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT track_uri FROM collection_tracks WHERE collection=?1 ORDER BY position",
            )?;
            stmt.query_map([&collection], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let json = serde_json::to_string(&entries)?;
        tx.execute(
            "UPDATE playlist_history_baselines SET observed_at=?2, entries=?3 WHERE collection=?1",
            params![collection, now, json],
        )?;
    }
    tx.execute(
        "DELETE FROM playlist_history_baselines WHERE observed_at < ?1",
        [cutoff],
    )?;
    tx.execute(
        "DELETE FROM playlist_history_events WHERE id IN (
            SELECT id FROM playlist_history_events e WHERE (SELECT COUNT(*) FROM playlist_history_events n WHERE n.collection=e.collection AND n.id>=e.id) > ?1
        )", [MAX_EVENTS_PER_LIST],
    )?;
    loop {
        let bytes: i64 = tx.query_row(
            "SELECT COALESCE((SELECT SUM(length(entries)) FROM playlist_history_baselines), 0) +
                    COALESCE((SELECT SUM(length(collection)+length(kind)+length(track_uri)+length(source)+32) FROM playlist_history_events), 0)",
            [], |row| row.get(0),
        )?;
        if bytes <= MAX_BYTES {
            break;
        }
        let oldest: Option<i64> =
            tx.query_row("SELECT MIN(id) FROM playlist_history_events", [], |row| {
                row.get(0)
            })?;
        if let Some(id) = oldest {
            tx.execute("DELETE FROM playlist_history_events WHERE id=?1", [id])?;
        } else {
            tx.execute("DELETE FROM playlist_history_baselines WHERE collection=(SELECT collection FROM playlist_history_baselines ORDER BY observed_at LIMIT 1)", [])?;
            if tx.query_row("SELECT COUNT(*) FROM playlist_history_baselines", [], |r| {
                r.get::<_, i64>(0)
            })? == 0
            {
                break;
            }
        }
    }
    Ok(())
}

pub fn clear(tx: &Transaction, collection: Option<&str>) -> Result<()> {
    if let Some(collection) = collection {
        tx.execute(
            "DELETE FROM playlist_history_events WHERE collection=?1",
            [collection],
        )?;
        tx.execute(
            "DELETE FROM playlist_history_baselines WHERE collection=?1",
            [collection],
        )?;
    } else {
        tx.execute_batch(
            "DELETE FROM playlist_history_events; DELETE FROM playlist_history_baselines;",
        )?;
    }
    Ok(())
}

/// Occurrence-aware diff. Identical URI occurrences are paired in order, then
/// a longest increasing subsequence identifies unchanged relative ordering.
pub fn diff(before: &[String], after: &[String]) -> Vec<Change> {
    let mut old_positions: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (i, uri) in before.iter().enumerate() {
        old_positions.entry(uri).or_default().push_back(i);
    }
    let mut matched = Vec::new();
    let mut added = Vec::new();
    for (new_i, uri) in after.iter().enumerate() {
        match old_positions
            .get_mut(uri.as_str())
            .and_then(VecDeque::pop_front)
        {
            Some(old_i) => matched.push((old_i, new_i, uri.clone())),
            None => added.push((new_i, uri.clone())),
        }
    }
    let keep: HashSet<usize> = lis_matches(&matched).into_iter().collect();
    let mut removed = Vec::new();
    for (i, pair) in matched.into_iter().enumerate() {
        if !keep.contains(&i) {
            removed.push((pair.0, pair.2.clone()));
            added.push((pair.1, pair.2));
        }
    }
    for (uri, positions) in old_positions {
        removed.extend(positions.into_iter().map(|i| (i, uri.to_string())));
    }
    let mut changes = Vec::new();
    let mut used_adds = vec![false; added.len()];
    for (old_i, uri) in removed {
        if let Some((n, (new_i, _))) = added
            .iter()
            .enumerate()
            .find(|(n, (new_i, candidate))| !used_adds[*n] && candidate == &uri && *new_i != old_i)
        {
            used_adds[n] = true;
            changes.push(Change {
                kind: ChangeKind::Moved,
                uri,
                from: Some(old_i),
                to: Some(*new_i),
            });
        } else {
            changes.push(Change {
                kind: ChangeKind::Removed,
                uri,
                from: Some(old_i),
                to: None,
            });
        }
    }
    for (n, (position, uri)) in added.into_iter().enumerate() {
        if !used_adds[n] {
            changes.push(Change {
                kind: ChangeKind::Added,
                uri,
                from: None,
                to: Some(position),
            });
        }
    }
    changes.sort_by_key(|c| c.from.or(c.to).unwrap_or(0));
    changes
}

fn lis_matches(pairs: &[(usize, usize, String)]) -> Vec<usize> {
    let mut tails: Vec<usize> = Vec::new();
    let mut tail_at: Vec<usize> = Vec::new();
    let mut previous = vec![None; pairs.len()];
    for (i, (old_i, _, _)) in pairs.iter().enumerate() {
        let slot = tails.partition_point(|v| *v < *old_i);
        if slot > 0 {
            previous[i] = Some(tail_at[slot - 1]);
        }
        if slot == tails.len() {
            tails.push(*old_i);
            tail_at.push(i);
        } else {
            tails[slot] = *old_i;
            tail_at[slot] = i;
        }
    }
    let mut result = Vec::new();
    let mut at = tail_at.last().copied();
    while let Some(i) = at {
        result.push(i);
        at = previous[i];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::library::Entry;
    use crate::engine::{CollectionId, Library, LibraryItem, LibraryMeta, TrackRow};

    fn uris(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn diff_reports_add_remove_and_positions() {
        let got = diff(&uris(&["a", "b", "c"]), &uris(&["a", "x", "c"]));
        assert_eq!(
            got,
            vec![
                Change {
                    kind: ChangeKind::Removed,
                    uri: "b".into(),
                    from: Some(1),
                    to: None
                },
                Change {
                    kind: ChangeKind::Added,
                    uri: "x".into(),
                    from: None,
                    to: Some(1)
                },
            ]
        );
    }

    #[test]
    fn diff_reports_reordering_as_a_move() {
        let got = diff(&uris(&["a", "b", "c"]), &uris(&["b", "a", "c"]));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ChangeKind::Moved);
        assert_eq!(got[0].uri, "b");
    }

    #[test]
    fn diff_pairs_duplicate_tracks_by_occurrence() {
        let got = diff(&uris(&["a", "a", "b"]), &uris(&["a", "b", "a", "a"]));
        assert_eq!(
            got.iter().filter(|c| c.kind == ChangeKind::Added).count(),
            1
        );
        assert_eq!(
            got.iter().filter(|c| c.kind == ChangeKind::Removed).count(),
            0
        );
    }

    #[test]
    fn recorded_events_reconstruct_an_older_version() {
        let events = vec![
            StoredChange {
                id: 1,
                at: 20,
                kind: ChangeKind::Removed,
                uri: "b".into(),
                from: Some(1),
                to: None,
                added_by: None,
                source: "spotify".into(),
            },
            StoredChange {
                id: 2,
                at: 20,
                kind: ChangeKind::Added,
                uri: "x".into(),
                from: None,
                to: Some(1),
                added_by: None,
                source: "spotify".into(),
            },
        ];
        assert_eq!(
            state_at(&uris(&["a", "x", "c"]), &events, 10),
            uris(&["a", "b", "c"])
        );
    }

    #[test]
    fn retention_caps_events_per_playlist_and_expires_old_data() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute_batch(
            "CREATE TABLE collection_tracks (collection TEXT, position INTEGER, track_uri TEXT);",
        )
        .unwrap();
        migration(&tx, 12).unwrap();
        let key = "spotify:playlist:test";
        record(&tx, key, None, &uris(&["a"]), 1, "spotify").unwrap();
        let mut before = uris(&["a"]);
        for i in 0..260 {
            let after = if i % 2 == 0 {
                uris(&["b"])
            } else {
                uris(&["a"])
            };
            record(&tx, key, Some(&before), &after, 2 + i, "spotify").unwrap();
            before = after;
        }
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM playlist_history_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, MAX_EVENTS_PER_LIST);
        prune(&tx, MAX_AGE_MS + 263).unwrap();
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM playlist_history_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let baselines: i64 = tx
            .query_row("SELECT COUNT(*) FROM playlist_history_baselines", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(baselines, 0);
    }

    #[test]
    fn global_retention_stays_within_twenty_megabytes() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        migration(&tx, 12).unwrap();
        let large_uri = "x".repeat(2 * 1024 * 1024);
        for i in 0..11 {
            let key = format!("spotify:playlist:{i}");
            record(
                &tx,
                &key,
                None,
                std::slice::from_ref(&large_uri),
                100 + i,
                "spotify",
            )
            .unwrap();
        }
        let bytes: i64 = tx.query_row(
            "SELECT COALESCE((SELECT SUM(length(entries)) FROM playlist_history_baselines),0)+COALESCE((SELECT SUM(length(collection)+length(kind)+length(track_uri)+length(source)+32) FROM playlist_history_events),0)",
            [], |r| r.get(0),
        ).unwrap();
        assert!(bytes <= MAX_BYTES, "{bytes} exceeds {MAX_BYTES}");
    }

    #[test]
    fn playlist_writes_record_only_new_revisions_and_mark_local_edits() {
        let db = crate::engine::cache::Db::open_in_memory().unwrap();
        let id = crate::engine::CollectionId::Playlist("spotify:playlist:revision-check".into());
        let entries = |uris: &[&str]| {
            uris.iter()
                .map(|uri| crate::engine::library::Entry::untimed(*uri))
                .collect::<Vec<_>>()
        };
        db.save_collection_entries(&id, Some(b"r1"), &entries(&["a", "b"]))
            .unwrap();
        db.save_collection_entries(&id, Some(b"r1"), &entries(&["a", "c"]))
            .unwrap();
        assert!(
            db.playlist_history(&id).unwrap().unwrap().events.is_empty(),
            "same revision is not another snapshot"
        );
        db.save_collection_entries(&id, Some(b"r2"), &entries(&["a", "c"]))
            .unwrap();
        assert!(
            db.playlist_history(&id).unwrap().unwrap().events.is_empty(),
            "new revision with equal content has no diff"
        );
        db.save_collection_entries(&id, None, &entries(&["c", "a"]))
            .unwrap();
        let snapshot = db.playlist_history(&id).unwrap().unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].source, "sen");
    }

    #[test]
    fn active_history_replaces_a_baseline_older_than_one_year() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute_batch(
            "CREATE TABLE collection_tracks (collection TEXT, position INTEGER, track_uri TEXT);",
        )
        .unwrap();
        migration(&tx, 12).unwrap();
        let key = "spotify:playlist:checkpoint";
        record(&tx, key, None, &uris(&["old"]), 1, "spotify").unwrap();
        tx.execute("INSERT INTO collection_tracks (collection, position, track_uri) VALUES (?1,0,'current')", [key]).unwrap();
        tx.execute("INSERT INTO playlist_history_events (collection, at, kind, track_uri, from_position, to_position, source) VALUES (?1,?2,'added','recent',NULL,1,'spotify')", rusqlite::params![key, MAX_AGE_MS + 4]).unwrap();
        prune(&tx, 2 * MAX_AGE_MS + 3).unwrap();
        let (at, json): (i64, String) = tx
            .query_row(
                "SELECT observed_at, entries FROM playlist_history_baselines WHERE collection=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(at, 2 * MAX_AGE_MS + 3);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&json).unwrap(),
            ["current"]
        );
        assert_eq!(
            tx.query_row("SELECT COUNT(*) FROM playlist_history_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    /// Offline UI fixture: `OPENIFY_SEED_DIR=<empty temp dir> cargo test --release seed_playlist_history_demo -- --ignored`.
    #[test]
    #[ignore = "writes the isolated OPENIFY_SEED_DIR cache used by the history screenshot"]
    fn seed_playlist_history_demo() {
        let dir = std::env::var_os("OPENIFY_SEED_DIR").expect("OPENIFY_SEED_DIR required");
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.db");
        assert!(!path.exists(), "seed directory must be empty");
        let db = crate::engine::cache::Db::open(&path).unwrap();
        let id = CollectionId::Playlist("spotify:playlist:historydemo".into());
        let item = LibraryItem {
            id: id.clone(),
            name: "Evening walk".into(),
            subtitle: crate::i18n::canon::songs(5),
            image: String::new(),
        };
        let mut meta = LibraryMeta::default();
        meta.creators.insert(id.key().to_string(), "demo".into());
        db.save_library(&Library {
            liked_count: 0,
            albums: Vec::new(),
            playlists: vec![item],
            artists: Vec::new(),
            meta,
        })
        .unwrap();
        let rows: Vec<TrackRow> = (1..=5)
            .map(|n| TrackRow {
                uri: format!("spotify:track:history{n}"),
                name: [
                    "Soft Lights",
                    "Blue Hour",
                    "Window Rain",
                    "Last Train",
                    "Quiet Streets",
                ][(n - 1) as usize]
                    .into(),
                artists: "North Avenue".into(),
                album: "After Dusk".into(),
                duration_ms: 210_000,
                image: String::new(),
                plays: None,
            })
            .collect();
        db.save_tracks(&rows).unwrap();
        let entries = |numbers: &[u8]| {
            numbers
                .iter()
                .map(|n| Entry::untimed(format!("spotify:track:history{n}")))
                .collect::<Vec<_>>()
        };
        db.save_collection_entries(&id, Some(b"revision-one"), &entries(&[1, 2, 3, 4]))
            .unwrap();
        if std::env::var_os("OPENIFY_TEST_HISTORY_EMPTY").is_none() {
            db.save_collection_entries(&id, Some(b"revision-two"), &entries(&[1, 5, 3]))
                .unwrap();
        }
        let expected = if std::env::var_os("OPENIFY_TEST_HISTORY_EMPTY").is_some() {
            0
        } else {
            3
        };
        assert_eq!(
            db.playlist_history(&id).unwrap().unwrap().events.len(),
            expected
        );
    }
}
