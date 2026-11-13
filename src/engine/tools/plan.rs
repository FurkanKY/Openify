//! Birleştir, böl ve kalıcı sıralama için saf planlayıcılar.
//!
//! Hiçbiri Spotify'a bir şey yazmaz: ne yapılacağını hesaplar. Yazma, arayüzün
//! mevcut `EditOp`'larla (`CreatePlaylist`, `RemoveFromPlaylist`,
//! `MovePlaylistItems`) kurduğu işlemlerle edit kuyruğundan gider.

use std::collections::{BTreeSet, HashMap};

use super::{
    super::{library::TrackRow, rules::fold},
    civil,
    dupes::{DedupeMode, dedupe},
};

// ── Birleştir ───────────────────────────────────────────────────────────

/// Birleştirmenin sonucu: yeni listenin şarkıları.
#[derive(Debug, Clone, PartialEq)]
pub struct MergePlan {
    pub rows: Vec<TrackRow>,
    /// "Yinelenenleri at" ile çıkarılan satır sayısı.
    pub dropped: usize,
}

/// Listeleri verilen sırayla art arda ekler; `mode`a göre yinelenenleri atar.
/// Kaynak listelere dokunulmaz (kopya).
pub fn merge(lists: &[&[TrackRow]], mode: DedupeMode) -> MergePlan {
    let all: Vec<TrackRow> = lists.iter().flat_map(|l| l.iter().cloned()).collect();
    let (rows, dropped) = dedupe(&all, mode);
    MergePlan { rows, dropped }
}

// ── Böl ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitBy {
    Artist,
    Album,
    /// Şarkının listeye EKLENDİĞİ yıl. Önbellekteki şarkı bilgisinde çıkış yılı
    /// yok (yalnız ad, sanatçı, albüm adı, süre); eklenme anı Spotify'dan gelir.
    AddedYear,
}

/// En az bu kadar şarkısı olmayan sanatçı / albüm grubu ayrı liste olmaz,
/// "diğer" grubuna gider (yüzlerce tek şarkılık liste açılmasın).
pub const SPLIT_MIN_GROUP: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct SplitGroup {
    /// Sanatçı adı, albüm adı ya da yıl; `other` grubunda boş.
    pub label: String,
    /// Küçük (ya da yılı bilinmeyen) grupların toplandığı grup.
    pub other: bool,
    pub rows: Vec<TrackRow>,
}

/// Birinci (ana) sanatçı: "A, B" → "A".
pub fn primary_artist(artists: &str) -> &str {
    artists.split(", ").next().unwrap_or("").trim()
}

/// `rows`u ölçüte göre gruplara böler: büyük gruptan küçüğe (eşitlikte ada
/// göre), "diğer" en sonda. Her şarkı tam bir grupta; sırası listedeki gibi.
/// `added`: şarkı adresi → eklenme anı (ms); yalnız `AddedYear` için.
pub fn split(
    rows: &[TrackRow],
    by: SplitBy,
    added: &HashMap<String, i64>,
    offset_minutes: i64,
) -> Vec<SplitGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, (String, Vec<TrackRow>)> = HashMap::new();
    let mut unknown: Vec<TrackRow> = Vec::new();
    for row in rows {
        let (key, label) = match by {
            SplitBy::Artist => {
                let name = primary_artist(&row.artists);
                if name.is_empty() {
                    unknown.push(row.clone());
                    continue;
                }
                (fold(name), name.to_string())
            }
            SplitBy::Album => {
                let album = row.album.trim();
                if album.is_empty() {
                    unknown.push(row.clone());
                    continue;
                }
                // Aynı adlı farklı albümler ("Greatest Hits") sanatçıyla ayrılır.
                (
                    format!(
                        "{}\u{1f}{}",
                        fold(album),
                        fold(primary_artist(&row.artists))
                    ),
                    album.to_string(),
                )
            }
            SplitBy::AddedYear => match added.get(&row.uri) {
                Some(at) => {
                    let year = civil::year_of(*at, offset_minutes).to_string();
                    (year.clone(), year)
                }
                None => {
                    unknown.push(row.clone());
                    continue;
                }
            },
        };
        groups
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key);
                (label, Vec::new())
            })
            .1
            .push(row.clone());
    }
    let min = if by == SplitBy::AddedYear {
        1
    } else {
        SPLIT_MIN_GROUP
    };
    let mut big: Vec<SplitGroup> = Vec::new();
    let mut other: Vec<TrackRow> = unknown;
    for key in order {
        let (label, rows) = groups.remove(&key).expect("anahtar sırada var");
        if rows.len() >= min {
            big.push(SplitGroup {
                label,
                other: false,
                rows,
            });
        } else {
            other.extend(rows);
        }
    }
    // Yıllar kronolojik (eskiden yeniye), ötekiler büyükten küçüğe.
    if by == SplitBy::AddedYear {
        big.sort_by(|a, b| a.label.cmp(&b.label));
    } else {
        big.sort_by(|a, b| {
            b.rows
                .len()
                .cmp(&a.rows.len())
                .then_with(|| fold(&a.label).cmp(&fold(&b.label)))
        });
    }
    // "Diğer" grubunda şarkılar listedeki özgün sıralarıyla kalsın.
    if !other.is_empty() {
        let index: HashMap<&str, usize> = rows
            .iter()
            .enumerate()
            .rev()
            .map(|(i, r)| (r.uri.as_str(), i))
            .collect();
        other.sort_by_key(|r| index.get(r.uri.as_str()).copied().unwrap_or(usize::MAX));
        big.push(SplitGroup {
            label: String::new(),
            other: true,
            rows: other,
        });
    }
    big
}

// ── Kalıcı sıralama ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortBy {
    Name,
    Artist,
    Album,
    Added,
    Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Asc,
    Desc,
}

/// İstenen sıra: `rows`un sıralanmış hâlindeki sırası (indeks dizisi). Eşit
/// anahtarlar listedeki sırasını korur (kararlı); eklenme anı bilinmeyenler
/// yönden bağımsız hep sonda.
pub fn target_order(
    rows: &[TrackRow],
    by: SortBy,
    dir: Dir,
    added: &HashMap<String, i64>,
) -> Vec<usize> {
    use std::cmp::Ordering;
    let mut ix: Vec<usize> = (0..rows.len()).collect();
    let text = |s: &str| fold(s);
    ix.sort_by(|&a, &b| {
        let (ra, rb) = (&rows[a], &rows[b]);
        let ord = match by {
            SortBy::Name => text(&ra.name)
                .cmp(&text(&rb.name))
                .then_with(|| text(&ra.artists).cmp(&text(&rb.artists))),
            SortBy::Artist => text(&ra.artists)
                .cmp(&text(&rb.artists))
                .then_with(|| text(&ra.album).cmp(&text(&rb.album)))
                .then_with(|| text(&ra.name).cmp(&text(&rb.name))),
            SortBy::Album => text(&ra.album)
                .cmp(&text(&rb.album))
                .then_with(|| text(&ra.artists).cmp(&text(&rb.artists))),
            SortBy::Duration => ra.duration_ms.cmp(&rb.duration_ms),
            SortBy::Added => {
                let (da, db) = (added.get(&ra.uri), added.get(&rb.uri));
                return match (da, db) {
                    (Some(x), Some(y)) => {
                        let o = x.cmp(y);
                        let o = if dir == Dir::Desc { o.reverse() } else { o };
                        o.then(a.cmp(&b))
                    }
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => a.cmp(&b),
                };
            }
        };
        let ord = if dir == Dir::Desc { ord.reverse() } else { ord };
        ord.then(a.cmp(&b))
    });
    ix
}

/// Tek öğe taşıması: `from`dan alıp sonuçta `to` indisinde olacak biçimde
/// (`EditOp::MovePlaylistItems` { length: 1 } ile aynı anlam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    pub from: usize,
    pub to: usize,
}

/// Taşımayı bir adres listesine uygular (arayüzün anında göstereceği sıra; her
/// taşımanın hangi şarkıyı götürdüğünü bulmak için).
pub fn simulate_move(list: &mut Vec<String>, from: usize, to: usize) -> bool {
    super::super::edits::apply_move(list, from, 1, to)
}

/// Listeyi `target` sırasına getiren en az sayıda tek öğe taşıması.
///
/// `target[k]`: hedef sırada k. satırın şimdiki indisi (`target_order`un
/// çıktısı). Yer değiştirmesi gerekmeyen en uzun artan altdizi (LIS) yerinde
/// kalır; geri kalan her öğe için tek taşıma: en az taşıma sayısı = n − LIS.
/// Taşımalar sırayla uygulanır (her biri öncekinin sonucuna göre).
pub fn plan_moves(target: &[usize]) -> Vec<Move> {
    let n = target.len();
    // rank[i]: şimdiki i. satırın hedef sıradaki yeri.
    let mut rank = vec![0usize; n];
    for (k, &i) in target.iter().enumerate() {
        rank[i] = k;
    }
    let stay = longest_increasing(&rank);
    let mut placed: BTreeSet<usize> = stay.iter().map(|&i| rank[i]).collect();
    // Taşınacaklar, hedef sırasıyla.
    let mut movers: Vec<usize> = (0..n)
        .map(|i| rank[i])
        .filter(|r| !placed.contains(r))
        .collect();
    movers.sort_unstable();
    // Simüle edilen liste: satırların hedef sıra numaraları.
    let mut cur: Vec<usize> = rank.clone();
    let mut moves = Vec::with_capacity(movers.len());
    for r in movers {
        let from = cur.iter().position(|&x| x == r).expect("öğe listede");
        cur.remove(from);
        // Yerleştirilmişler arasında r'den küçük en büyük sıra numaralısının hemen arkası
        // (yerleştirilmişler listede hep artan sırada, ötekiler aralarına serpilmiş).
        let after = match placed.range(..r).next_back() {
            Some(pred) => cur.iter().position(|x| x == pred).map_or(0, |i| i + 1),
            None => 0,
        };
        cur.insert(after, r);
        placed.insert(r);
        if after != from {
            moves.push(Move { from, to: after });
        }
    }
    moves
}

/// `plan_moves(target).len()`, planı kurmadan: n eksi en uzun artan altdizi
/// (O(n log n)). Büyük listede "çok taşıma" kararı için ucuz.
pub fn count_moves(target: &[usize]) -> usize {
    let n = target.len();
    let mut rank = vec![0usize; n];
    for (k, &i) in target.iter().enumerate() {
        rank[i] = k;
    }
    n - longest_increasing(&rank).len()
}

/// `values`ın en uzun kesin artan altdizisinin indisleri (artan sırada).
fn longest_increasing(values: &[usize]) -> Vec<usize> {
    // Sabırlı sıralama: tails[k] = uzunluğu k+1 olan altdizinin en küçük son değerinin indisi.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; values.len()];
    for (i, &v) in values.iter().enumerate() {
        let pos = tails.partition_point(|&t| values[t] < v);
        if pos > 0 {
            prev[i] = Some(tails[pos - 1]);
        }
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(i) = at {
        out.push(i);
        at = prev[i];
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::edits::apply_move;

    fn row(n: u32, name: &str, artists: &str, album: &str, ms: u32) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{n:022}"),
            name: name.into(),
            artists: artists.into(),
            album: album.into(),
            duration_ms: ms,
            image: String::new(),
            plays: None,
        }
    }

    fn uris(rows: &[TrackRow]) -> Vec<String> {
        rows.iter().map(|r| r.uri.clone()).collect()
    }

    // ── Birleştir ───────────────────────────────────────────────────────

    #[test]
    fn merge_concatenates_in_order_and_leaves_the_sources_alone() {
        let a = vec![row(1, "A", "X", "Y", 1), row(2, "B", "X", "Y", 1)];
        let b = vec![row(3, "C", "X", "Y", 1)];
        let plan = merge(&[&a, &b], DedupeMode::Off);
        assert_eq!(
            uris(&plan.rows),
            [a[0].uri.clone(), a[1].uri.clone(), b[0].uri.clone()]
        );
        assert_eq!(plan.dropped, 0);
        assert_eq!((a.len(), b.len()), (2, 1));
    }

    #[test]
    fn merge_can_drop_the_same_track_or_versions() {
        let s1 = row(1, "Ayna", "Deniz", "Y", 180_000);
        let remaster = row(2, "Ayna - Remastered", "Deniz", "Y", 181_000);
        let other = row(3, "Limanda", "Deniz", "Y", 200_000);
        let a = vec![s1.clone(), other.clone()];
        let b = vec![s1.clone(), remaster.clone()];
        let same = merge(&[&a, &b], DedupeMode::SameTrack);
        assert_eq!((same.rows.len(), same.dropped), (3, 1));
        let versions = merge(&[&a, &b], DedupeMode::Versions);
        assert_eq!(uris(&versions.rows), [s1.uri.clone(), other.uri.clone()]);
        assert_eq!(versions.dropped, 2);
        let none = merge(&[&a, &b], DedupeMode::Off);
        assert_eq!(none.rows.len(), 4);
    }

    // ── Böl ─────────────────────────────────────────────────────────────

    fn library() -> Vec<TrackRow> {
        vec![
            row(1, "a1", "Mavi Orman", "Uzun Yol", 1),
            row(2, "k1", "Kuzey", "Kuzey Işıkları", 1),
            row(3, "a2", "Mavi Orman, Konuk", "Uzun Yol", 1),
            row(4, "d1", "Deniz Aydın", "Sessiz Ev", 1),
            row(5, "a3", "Mavi Orman", "Başka Albüm", 1),
            row(6, "k2", "Kuzey", "Kuzey Işıkları", 1),
            row(7, "k3", "Kuzey", "Kuzey Işıkları", 1),
        ]
    }

    #[test]
    fn split_by_artist_uses_the_first_artist_and_collects_small_groups() {
        let groups = split(&library(), SplitBy::Artist, &HashMap::new(), 0);
        let shown: Vec<(String, bool, usize)> = groups
            .iter()
            .map(|g| (g.label.clone(), g.other, g.rows.len()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Kuzey".into(), false, 3),
                ("Mavi Orman".into(), false, 3),
                (String::new(), true, 1)
            ]
        );
        // Her şarkı tam bir grupta.
        let total: usize = groups.iter().map(|g| g.rows.len()).sum();
        assert_eq!(total, 7);
        assert_eq!(groups[2].rows[0].name, "d1");
    }

    #[test]
    fn split_by_album_separates_same_named_albums_of_different_artists() {
        let mut rows = library();
        // Aynı ada sahip iki albüm.
        for n in 10..13 {
            rows.push(row(n, "x", "Biri", "Greatest Hits", 1));
        }
        for n in 13..16 {
            rows.push(row(n, "y", "Öteki", "Greatest Hits", 1));
        }
        let groups = split(&rows, SplitBy::Album, &HashMap::new(), 0);
        let hits: Vec<_> = groups
            .iter()
            .filter(|g| g.label == "Greatest Hits")
            .collect();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|g| g.rows.len() == 3));
        // Kuzey Işıkları (3) ve Uzun Yol (2 → diğer) …
        assert!(groups.iter().any(|g| g.label == "Kuzey Işıkları"));
        assert!(!groups.iter().any(|g| g.label == "Uzun Yol"));
        assert!(groups.last().unwrap().other);
    }

    #[test]
    fn split_by_added_year_is_chronological_and_unknown_dates_go_to_other() {
        let rows = library();
        let ms = |y: i32| {
            ((y as i64 - 1970) * 365 + (y as i64 - 1969) / 4) * 86_400_000 + 86_400_000 * 40
        };
        let added: HashMap<String, i64> = [
            (rows[0].uri.clone(), ms(2024)),
            (rows[1].uri.clone(), ms(2022)),
            (rows[2].uri.clone(), ms(2024)),
            (rows[3].uri.clone(), ms(2022)),
            // 4, 5, 6: tarih yok
        ]
        .into();
        let groups = split(&rows, SplitBy::AddedYear, &added, 0);
        let shown: Vec<(String, usize)> = groups
            .iter()
            .map(|g| (g.label.clone(), g.rows.len()))
            .collect();
        assert_eq!(
            shown,
            [("2022".into(), 2), ("2024".into(), 2), (String::new(), 3)]
        );
        assert!(groups[2].other);
    }

    #[test]
    fn nothing_to_split_gives_no_groups() {
        assert!(split(&[], SplitBy::Artist, &HashMap::new(), 0).is_empty());
    }

    // ── Sıralama ────────────────────────────────────────────────────────

    #[test]
    fn target_order_sorts_by_each_key_and_is_stable() {
        let rows = vec![
            row(1, "b", "Z", "m", 300),
            row(2, "A", "y", "n", 100),
            row(3, "c", "Z", "m", 200),
            row(4, "A", "x", "o", 100),
        ];
        let names = |o: &[usize]| o.iter().map(|&i| rows[i].name.clone()).collect::<Vec<_>>();
        let none = HashMap::new();
        assert_eq!(
            names(&target_order(&rows, SortBy::Name, Dir::Asc, &none)),
            ["A", "A", "b", "c"]
        );
        // "A" iki tane: ikinci anahtar sanatçı (x < y).
        assert_eq!(
            target_order(&rows, SortBy::Name, Dir::Asc, &none),
            [3, 1, 0, 2]
        );
        assert_eq!(
            target_order(&rows, SortBy::Name, Dir::Desc, &none),
            [2, 0, 1, 3]
        );
        assert_eq!(
            target_order(&rows, SortBy::Artist, Dir::Asc, &none),
            [3, 1, 0, 2]
        );
        assert_eq!(
            target_order(&rows, SortBy::Album, Dir::Asc, &none),
            [0, 2, 1, 3]
        );
        // Süre: eşitler listedeki sırasını korur (1 önce 3).
        assert_eq!(
            target_order(&rows, SortBy::Duration, Dir::Asc, &none),
            [1, 3, 2, 0]
        );
        assert_eq!(
            target_order(&rows, SortBy::Duration, Dir::Desc, &none),
            [0, 2, 1, 3]
        );
    }

    #[test]
    fn sorting_ignores_case_and_turkish_dotted_capitals() {
        let rows = vec![
            row(1, "Zeytin", "x", "a", 1),
            row(2, "ırmak", "x", "a", 1),
            row(3, "İkinci", "x", "a", 1),
            row(4, "ada", "x", "a", 1),
        ];
        let order = target_order(&rows, SortBy::Name, Dir::Asc, &HashMap::new());
        let names: Vec<&str> = order.iter().map(|&i| rows[i].name.as_str()).collect();
        assert_eq!(names[0], "ada", "{names:?}");
    }

    #[test]
    fn sort_by_added_puts_unknown_dates_last_in_both_directions() {
        let rows = vec![
            row(1, "a", "x", "a", 1),
            row(2, "b", "x", "a", 1),
            row(3, "c", "x", "a", 1),
            row(4, "d", "x", "a", 1),
        ];
        let added: HashMap<String, i64> = [
            (rows[0].uri.clone(), 300),
            (rows[2].uri.clone(), 100),
            (rows[3].uri.clone(), 200),
        ]
        .into();
        assert_eq!(
            target_order(&rows, SortBy::Added, Dir::Asc, &added),
            [2, 3, 0, 1]
        );
        assert_eq!(
            target_order(&rows, SortBy::Added, Dir::Desc, &added),
            [0, 3, 2, 1]
        );
    }

    fn apply(list: &mut Vec<usize>, moves: &[Move]) {
        for m in moves {
            assert!(apply_move(list, m.from, 1, m.to), "geçersiz taşıma {m:?}");
        }
    }

    fn lis_len(v: &[usize]) -> usize {
        longest_increasing(v).len()
    }

    #[test]
    fn moves_sort_the_list_and_never_use_more_than_n_minus_lis() {
        // Deterministik sahte rastgele (xorshift), çeşitli boy ve karışıklık.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for n in [0usize, 1, 2, 3, 5, 8, 13, 30, 64, 120] {
            for _ in 0..25 {
                // Rastgele hedef sırası (Fisher–Yates).
                let mut target: Vec<usize> = (0..n).collect();
                for i in (1..n).rev() {
                    let j = (next() % (i as u64 + 1)) as usize;
                    target.swap(i, j);
                }
                let moves = plan_moves(&target);
                assert_eq!(count_moves(&target), moves.len(), "n={n}");
                // Başlangıç: satırların hedef sıra numaraları.
                let mut rank = vec![0usize; n];
                for (k, &i) in target.iter().enumerate() {
                    rank[i] = k;
                }
                let want = n - lis_len(&rank);
                assert_eq!(moves.len(), want, "n={n} target={target:?}");
                let mut list = rank.clone();
                apply(&mut list, &moves);
                assert_eq!(
                    list,
                    (0..n).collect::<Vec<_>>(),
                    "n={n} target={target:?} moves={moves:?}"
                );
                assert!(moves.iter().all(|m| m.from != m.to));
            }
        }
    }

    #[test]
    fn an_already_sorted_list_needs_no_moves_and_one_displaced_row_needs_one() {
        assert!(plan_moves(&[0, 1, 2, 3, 4]).is_empty());
        assert!(plan_moves(&[]).is_empty());
        // Hedef sırada 3. satır en başa: tek taşıma.
        let moves = plan_moves(&[3, 0, 1, 2]);
        assert_eq!(moves, [Move { from: 3, to: 0 }]);
        // Tersine çevrilmiş 5'lik: n − LIS = 4.
        assert_eq!(plan_moves(&[4, 3, 2, 1, 0]).len(), 4);
    }

    #[test]
    fn real_sort_end_to_end_orders_the_uris() {
        let rows = vec![
            row(1, "c", "x", "a", 1),
            row(2, "a", "x", "a", 1),
            row(3, "d", "x", "a", 1),
            row(4, "b", "x", "a", 1),
        ];
        let target = target_order(&rows, SortBy::Name, Dir::Asc, &HashMap::new());
        let moves = plan_moves(&target);
        let mut list: Vec<String> = uris(&rows);
        for m in &moves {
            assert!(apply_move(&mut list, m.from, 1, m.to));
        }
        let want: Vec<String> = target.iter().map(|&i| rows[i].uri.clone()).collect();
        assert_eq!(list, want);
    }

    /// Risk ölçümü: büyük listede sıralama planı ve 500 taşımanın liste yazımı
    /// (taşıma başına liste kopyası + uygulama: motorun `patch_local` yolu; arayüz
    /// `manage::commit`te tek kopyayla yazar, daha ucuz) kullanıcıyı bekletmez.
    /// Çıta (gevşek, makine farkı için): hesap < 1 sn, yazım < 3 sn; ölçülen
    /// değerler `--nocapture` ile yazılır.
    #[test]
    fn planning_scales_to_a_five_thousand_row_list() {
        use std::time::Instant;
        let n = 5000usize;
        let rows: Vec<TrackRow> = (0..n)
            .map(|i| row(i as u32, &format!("{:05}", (i * 7919) % n), "x", "y", 1))
            .collect();
        // Üç denemenin en iyisi: makine başka derlemelerle doluyken tek ölçüm sapar.
        let (mut order_ms, mut plan_ms, mut ui_ms) = (f64::MAX, f64::MAX, f64::MAX);
        let mut moves = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            let order = target_order(&rows, SortBy::Name, Dir::Asc, &HashMap::new());
            order_ms = order_ms.min(t.elapsed().as_secs_f64() * 1000.);
            let t = Instant::now();
            moves = plan_moves(&order);
            plan_ms = plan_ms.min(t.elapsed().as_secs_f64() * 1000.);
            let t = Instant::now();
            let mut list = rows.clone();
            for m in moves.iter().take(500) {
                let mut copy = list.clone();
                assert!(apply_move(&mut copy, m.from, 1, m.to));
                list = copy;
            }
            ui_ms = ui_ms.min(t.elapsed().as_secs_f64() * 1000.);
        }
        println!(
            "ölçüm: 5000 satır: hedef sıra {order_ms:.1} ms, taşıma planı {plan_ms:.1} ms ({} taşıma), 500 taşımanın arayüz yazımı {ui_ms:.1} ms",
            moves.len()
        );
        assert!(order_ms < 1000. && plan_ms < 1000. && ui_ms < 3000.);
    }
}
