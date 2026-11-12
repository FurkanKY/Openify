//! Yinelenen şarkı bulucu (saf).
//!
//! İki tür yinelenen:
//!   - `SameTrack`: aynı şarkı adresi (`spotify:track:…`) listede birden çok kez.
//!   - `Versions`: aynı şarkının farklı sürümleri (Remaster, Live, Radio Edit,
//!     feat. …): karıştırma kurallarındaki "tek sürüm" anahtarı
//!     (`rules::song_key`) ve süre sağlaması (`rules::durations_compatible`)
//!     aynen yeniden kullanılır; "bu iki kayıt aynı şarkı mı" sorusunun tek cevabı
//!     var. Remix, Acoustic, Instrumental, Sped Up, Part 2 sürüm sayılmaz
//!     (`rules.rs` muhafazakâr).
//!
//! Her grupta tam bir satır "kalır", ötekiler kaldırılabilir. Kalan: başlığı en
//! sade olan (en kısa ad: "Song" > "Song - 2011 Remaster"), eşitlikte listede en
//! önde olan. Hangi satırların gideceğine kullanıcı karar verir; buradaki yalnız
//! varsayılan.

use std::collections::{HashMap, HashSet};

use super::super::{
    library::TrackRow,
    rules::{durations_compatible, song_key},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DupKind {
    /// Aynı adres, birden çok kopya.
    SameTrack,
    /// Farklı sürümler (aynı adresli kopyalar da içerebilir).
    Versions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DupItem {
    /// Satırın listedeki sırası (0'dan).
    pub index: usize,
    pub row: TrackRow,
    /// Grupta kalacak satır mı.
    pub keep: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DupGroup {
    pub kind: DupKind,
    /// Listedeki sıraya göre; tam biri `keep`.
    pub items: Vec<DupItem>,
}

impl DupGroup {
    /// Kaldırılabilir (kalmayan) satır sayısı.
    pub fn removable(&self) -> usize {
        self.items.iter().filter(|i| !i.keep).count()
    }
}

/// Listedeki yinelenen gruplar, ilk satırlarının sırasıyla.
pub fn find(tracks: &[TrackRow]) -> Vec<DupGroup> {
    // Anahtarın ilk görüldüğü sıra korunur (çıktı sırası deterministik).
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, row) in tracks.iter().enumerate() {
        let key = match song_key(&row.name, &row.artists) {
            Some(k) => format!("s:{k}"),
            // Anahtarı çıkmayan (çok genel ad, sanatçısız) yalnız aynı adresle eşleşir.
            None => format!("u:{}", row.uri),
        };
        by_key
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(i);
    }
    let mut groups = Vec::new();
    for key in &order {
        let members = &by_key[key];
        if members.len() < 2 {
            continue;
        }
        for cluster in cluster_by_duration(tracks, members) {
            if cluster.len() < 2 {
                continue;
            }
            groups.push(make_group(tracks, &cluster));
        }
    }
    groups.sort_by_key(|g| g.items[0].index);
    groups
}

/// Aynı anahtarlı satırları süreye göre kümele: biri ötekinin iki katından
/// uzunsa farklı kayıt. Aynı adresli satırlar her zaman aynı kümede.
fn cluster_by_duration(tracks: &[TrackRow], members: &[usize]) -> Vec<Vec<usize>> {
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for &i in members {
        let row = &tracks[i];
        let slot = clusters
            .iter()
            .position(|c| c.iter().any(|&j| tracks[j].uri == row.uri))
            .or_else(|| {
                clusters
                    .iter()
                    .position(|c| durations_compatible(tracks[c[0]].duration_ms, row.duration_ms))
            });
        match slot {
            Some(s) => clusters[s].push(i),
            None => clusters.push(vec![i]),
        }
    }
    clusters
}

fn make_group(tracks: &[TrackRow], members: &[usize]) -> DupGroup {
    let same = members
        .iter()
        .all(|&i| tracks[i].uri == tracks[members[0]].uri);
    // Kalan: en kısa ad, eşitlikte en öndeki (üyeler zaten sıralı).
    let keep = members
        .iter()
        .copied()
        .min_by_key(|&i| (tracks[i].name.chars().count(), i))
        .unwrap_or(members[0]);
    DupGroup {
        kind: if same {
            DupKind::SameTrack
        } else {
            DupKind::Versions
        },
        items: members
            .iter()
            .map(|&i| DupItem {
                index: i,
                row: tracks[i].clone(),
                keep: i == keep,
            })
            .collect(),
    }
}

/// Kaldırılacak satırlar (liste sırası, satır): her grubun kalmayanları, ama
/// `spared` (kullanıcının işaretini kaldırdığı sıralar) hariç. Listedeki
/// sırayla döner (çağıran gerekirse çevirir).
pub fn removals(groups: &[DupGroup], spared: &HashSet<usize>) -> Vec<(usize, TrackRow)> {
    let mut out: Vec<(usize, TrackRow)> = groups
        .iter()
        .flat_map(|g| g.items.iter())
        .filter(|i| !i.keep && !spared.contains(&i.index))
        .map(|i| (i.index, i.row.clone()))
        .collect();
    out.sort_by_key(|(i, _)| *i);
    out
}

/// Birleştirmede "yinelenenleri at" seçeneği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum DedupeMode {
    #[default]
    Off,
    /// Aynı adresli kopyalar (ilki kalır).
    SameTrack,
    /// Aynı adres ve sürümler (gruptaki `keep` kalır).
    Versions,
}

/// `rows`tan seçeneğe göre yinelenenleri çıkarılmış kopya; çıkarılan sayı.
/// Kalanların sırası korunur.
pub fn dedupe(rows: &[TrackRow], mode: DedupeMode) -> (Vec<TrackRow>, usize) {
    let drop: HashSet<usize> = match mode {
        DedupeMode::Off => return (rows.to_vec(), 0),
        DedupeMode::SameTrack => {
            let mut seen: HashSet<&str> = HashSet::new();
            rows.iter()
                .enumerate()
                .filter(|(_, r)| !seen.insert(r.uri.as_str()))
                .map(|(i, _)| i)
                .collect()
        }
        DedupeMode::Versions => find(rows)
            .iter()
            .flat_map(|g| g.items.iter())
            .filter(|i| !i.keep)
            .map(|i| i.index)
            .collect(),
    };
    let kept = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, r)| r.clone())
        .collect();
    (kept, drop.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u32, name: &str, artists: &str, ms: u32) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{n:022}"),
            name: name.into(),
            artists: artists.into(),
            album: "Albüm".into(),
            duration_ms: ms,
            image: String::new(),
        }
    }

    fn same_uri(mut r: TrackRow, n: u32) -> TrackRow {
        r.uri = format!("spotify:track:{n:022}");
        r
    }

    #[test]
    fn a_list_without_duplicates_has_no_groups() {
        let rows = [
            row(1, "Sessiz Gece", "Mavi Orman", 200_000),
            row(2, "Ayna", "Deniz Aydın", 180_000),
        ];
        assert!(find(&rows).is_empty());
        assert!(find(&[]).is_empty());
    }

    #[test]
    fn the_same_uri_twice_is_a_same_track_group_and_the_first_copy_stays() {
        let a = row(1, "Sessiz Gece", "Mavi Orman", 200_000);
        let rows = [
            a.clone(),
            row(2, "Ayna", "Deniz Aydın", 180_000),
            a.clone(),
            a,
        ];
        let groups = find(&rows);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, DupKind::SameTrack);
        let kept: Vec<_> = groups[0]
            .items
            .iter()
            .filter(|i| i.keep)
            .map(|i| i.index)
            .collect();
        assert_eq!(kept, [0]);
        assert_eq!(groups[0].removable(), 2);
        let gone: Vec<usize> = removals(&groups, &HashSet::new())
            .iter()
            .map(|(i, _)| *i)
            .collect();
        assert_eq!(gone, [2, 3]);
    }

    #[test]
    fn versions_of_one_song_group_together_and_the_plainest_title_stays() {
        let rows = [
            row(
                1,
                "Bring Me to Life - 2011 Remaster",
                "Evanescence",
                235_000,
            ),
            row(2, "Sessiz Gece", "Mavi Orman", 200_000),
            row(3, "Bring Me to Life", "Evanescence", 236_000),
            row(4, "Bring Me to Life (Live)", "Evanescence", 250_000),
        ];
        let groups = find(&rows);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.kind, DupKind::Versions);
        assert_eq!(
            g.items.iter().map(|i| i.index).collect::<Vec<_>>(),
            [0, 2, 3]
        );
        let kept: Vec<_> = g.items.iter().filter(|i| i.keep).map(|i| i.index).collect();
        assert_eq!(kept, [2], "en sade başlık kalır");
    }

    #[test]
    fn remixes_and_other_artists_are_not_versions() {
        let rows = [
            row(1, "Aurora", "Kuzey", 200_000),
            row(2, "Aurora (Remix)", "Kuzey", 200_000),
            row(3, "Aurora", "Başka Biri", 200_000),
        ];
        assert!(find(&rows).is_empty(), "Remix ve başka sanatçı ayrı şarkı");
    }

    #[test]
    fn very_different_lengths_are_different_recordings() {
        // Aynı ad ve sanatçı ama biri iki katından uzun (ör. 10 dk'lık canlı kayıt).
        let rows = [
            row(1, "Aurora", "Kuzey", 200_000),
            row(2, "Aurora", "Kuzey", 700_000),
        ];
        assert!(find(&rows).is_empty());
        // Aynı adresli kopyalar süre ne olursa olsun yine yinelenen.
        let again = [
            rows[0].clone(),
            rows[1].clone(),
            same_uri(rows[1].clone(), 2),
        ];
        let groups = find(&again);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].items.iter().map(|i| i.index).collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn titles_without_a_song_key_only_match_by_address() {
        // Çok genel ad ("Track 1") anahtar üretmez: farklı adresliler eşlenmez,
        // aynı adresli kopya yine bulunur.
        let a = row(1, "Track 1", "Biri", 100_000);
        let rows = [a.clone(), row(2, "Track 1", "Biri", 100_000), a];
        let groups = find(&rows);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, DupKind::SameTrack);
        assert_eq!(
            groups[0].items.iter().map(|i| i.index).collect::<Vec<_>>(),
            [0, 2]
        );
    }

    #[test]
    fn mixed_group_with_exact_copies_and_a_version() {
        let a = row(1, "Ayna", "Deniz Aydın", 180_000);
        let rows = [
            a.clone(),
            row(2, "Ayna - Remastered 2020", "Deniz Aydın", 181_000),
            a,
        ];
        let groups = find(&rows);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, DupKind::Versions);
        assert_eq!(groups[0].removable(), 2);
        assert!(groups[0].items[0].keep);
    }

    #[test]
    fn removals_skip_what_the_user_unticked() {
        let a = row(1, "Ayna", "Deniz Aydın", 180_000);
        let rows = [a.clone(), a.clone(), a];
        let groups = find(&rows);
        let spared: HashSet<usize> = [1].into();
        let gone: Vec<usize> = removals(&groups, &spared).iter().map(|(i, _)| *i).collect();
        assert_eq!(gone, [2]);
    }

    #[test]
    fn groups_come_in_list_order() {
        let a = row(1, "Ayna", "Deniz Aydın", 180_000);
        let b = row(2, "Limanda", "Deniz Aydın", 200_000);
        let rows = [b.clone(), a.clone(), row(3, "Aurora", "Kuzey", 1), a, b];
        let groups = find(&rows);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].items[0].row.name, "Limanda");
        assert_eq!(groups[1].items[0].row.name, "Ayna");
    }

    #[test]
    fn dedupe_modes() {
        let a = row(1, "Ayna", "Deniz Aydın", 180_000);
        let v = row(2, "Ayna - Remastered 2020", "Deniz Aydın", 181_000);
        let b = row(3, "Limanda", "Deniz Aydın", 200_000);
        let rows = vec![a.clone(), v.clone(), b.clone(), a.clone()];
        assert_eq!(dedupe(&rows, DedupeMode::Off), (rows.clone(), 0));
        let (same, n) = dedupe(&rows, DedupeMode::SameTrack);
        assert_eq!((same.len(), n), (3, 1));
        assert_eq!(
            same.iter().map(|r| r.uri.clone()).collect::<Vec<_>>(),
            [a.uri.clone(), v.uri.clone(), b.uri.clone()]
        );
        let (versions, n) = dedupe(&rows, DedupeMode::Versions);
        assert_eq!(n, 2);
        assert_eq!(versions, [a, b]);
    }

    /// Risk ölçümü: 10 bin satırlık listede yinelenen bulma. Çıta: < 500 ms.
    #[test]
    fn finding_duplicates_in_ten_thousand_rows_is_fast() {
        let rows: Vec<TrackRow> = (0..10_000u32)
            .map(|i| {
                // Her 5. satır bir öncekinin Remaster sürümü.
                if i % 5 == 4 {
                    row(
                        i,
                        &format!("Şarkı {} - 2011 Remaster", i - 1),
                        "Sanatçı",
                        200_000,
                    )
                } else {
                    row(i, &format!("Şarkı {i}"), "Sanatçı", 200_000)
                }
            })
            .collect();
        let t = std::time::Instant::now();
        let groups = find(&rows);
        let ms = t.elapsed().as_secs_f64() * 1000.;
        println!(
            "ölçüm: 10000 satırda yinelenen bulma {ms:.1} ms ({} küme)",
            groups.len()
        );
        assert_eq!(groups.len(), 2000);
        assert!(ms < 500., "{ms} ms");
    }
}
