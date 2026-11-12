//! Kütüphane ağacının saf mantığı (gpui yok, testlenebilir): klasörler,
//! sabitleme ve sıralama.
//!
//! Girdi düz bir düğüm listesi (liste, albüm, klasör; her birinin üst klasörü,
//! sıralama anahtarı ve sabitlenme sırası), çıktı ekranda görünecek sıra ve
//! girinti derinliği. Arayüz (`shelf.rs`) düğümleri `Library`den kurar,
//! çıktıyı kartlara çevirir.
//!
//! Kurallar:
//! - Kardeşler arasında önce sabitlenenler (sabitlenme sırasıyla), sonra
//!   sıralama anahtarı, eşitlikte kütüphane sırası.
//! - Klasörün anahtarı, içindeki en önde gelen öğenin anahtarıdır ("son
//!   çalınan" / "eklenme": içindekilerin en yenisi); ya da kendi anahtarı
//!   (ad, yaratıcı). İçi boş klasör sona gider.
//! - Kökte sabitlenen öğeler (klasörlerin içindekiler de) en üstte ayrı bir
//!   blok olarak çıkar (`lifted`); kökteki sabitli öğe ağaçta ayrıca yinelenmez,
//!   klasörün içindekiler yerinde de durur (klasörün içeriği eksik kalmasın).
//! - Üst klasörü bilinmeyen düğüm kökte sayılır (kütüphane tazelenirken
//!   klasör kaybolmuş olabilir); döngü sonsuza gitmez.

use std::collections::{HashMap, HashSet};

/// Sıralama anahtarı: (grup, sayı, yazı) sözlük sırasıyla. Grup 0 en öne,
/// büyük gruplar sona gider ("hiç çalınmamış", "tarihi bilinmeyen" vb.).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub group: u8,
    pub num: i64,
    pub text: String,
}

impl Key {
    pub fn new(group: u8, num: i64, text: impl Into<String>) -> Self {
        Self {
            group,
            num,
            text: text.into(),
        }
    }

    /// En sona giden anahtar (içi boş klasör).
    pub fn last() -> Self {
        Self::new(u8::MAX, 0, "")
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    /// Liste / albüm / sanatçı anahtarı ya da klasörün işareti.
    pub id: String,
    /// İçinde durduğu klasörün `id`si; kökteyse `None`.
    pub parent: Option<String>,
    pub folder: bool,
    pub key: Key,
    /// Sabitlenme sırası (0 = en eski sabitlenen); sabit değilse `None`.
    pub pin: Option<usize>,
    /// Kütüphane sırası (eşitlikte bozulmasın).
    pub seq: usize,
}

/// Ekranda bir satır / kart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    /// `nodes` içindeki yeri.
    pub ix: usize,
    /// Girinti: kökte 0, klasörün içinde 1 …
    pub depth: usize,
    /// Kökün en üstündeki sabitli öğeler bloğundan mı (yerinde değil)?
    pub lifted: bool,
}

pub struct Options<'a> {
    /// Hangi klasörün içeriği gösterilecek (ızgarada klasöre girilince);
    /// `None`: kök.
    pub root: Option<&'a str>,
    /// Klasörler açık olunca içeriği satır araları çıksın mı (liste görünümü)?
    /// `Some(f)`: `f(klasör)` açıksa içi hemen altında; `None`: yalnız
    /// doğrudan çocuklar (ızgara: klasöre girilir).
    pub is_open: Option<&'a dyn Fn(&str) -> bool>,
    /// Kökte sabitlileri en üstte ayrı blok yap.
    pub lift_pins: bool,
    /// Klasör anahtarı içindekilerin en önde geleninden mi (`true`), kendi
    /// `key`inden mi?
    pub folders_by_children: bool,
}

/// Bir klasörün altındaki derinlik sınırı (bozuk bir ağaçta sonsuz döngü olmasın).
const MAX_DEPTH: usize = 32;

/// Ekranda görünecek düğümler, sırasıyla.
pub fn layout(nodes: &[Node], opts: &Options) -> Vec<Placed> {
    let folder_ids: HashSet<&str> = nodes
        .iter()
        .filter(|n| n.folder)
        .map(|n| n.id.as_str())
        .collect();
    fn parent_of<'a>(n: &'a Node, folders: &HashSet<&str>) -> Option<&'a str> {
        n.parent
            .as_deref()
            .filter(|p| folders.contains(p) && *p != n.id)
    }
    let mut children: HashMap<Option<&str>, Vec<usize>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        children
            .entry(parent_of(n, &folder_ids))
            .or_default()
            .push(i);
    }

    let mut keys: Vec<Key> = nodes.iter().map(|n| n.key.clone()).collect();
    if opts.folders_by_children {
        for (i, n) in nodes.iter().enumerate().filter(|(_, n)| n.folder) {
            keys[i] = best_below(nodes, &children, &keys, &n.id, 0).unwrap_or_else(Key::last);
        }
    }
    for list in children.values_mut() {
        list.sort_by(|&a, &b| {
            let (na, nb) = (&nodes[a], &nodes[b]);
            na.pin
                .is_none()
                .cmp(&nb.pin.is_none())
                .then(na.pin.cmp(&nb.pin))
                .then_with(|| keys[a].cmp(&keys[b]))
                .then(na.seq.cmp(&nb.seq))
        });
    }

    // Kök: bilinmeyen klasör adı verilmişse kök gibi davran.
    let root = opts.root.filter(|r| folder_ids.contains(r));
    let mut out = Vec::new();
    let lift = opts.lift_pins && root.is_none();
    if lift {
        let mut pinned: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.folder && n.pin.is_some())
            .map(|(i, _)| i)
            .collect();
        pinned.sort_by_key(|&i| (nodes[i].pin, nodes[i].seq));
        out.extend(pinned.into_iter().map(|ix| Placed {
            ix,
            depth: 0,
            lifted: true,
        }));
    }
    walk(nodes, &children, root, 0, opts.is_open, lift, &mut out);
    out
}

fn walk(
    nodes: &[Node],
    children: &HashMap<Option<&str>, Vec<usize>>,
    parent: Option<&str>,
    depth: usize,
    is_open: Option<&dyn Fn(&str) -> bool>,
    skip_pinned_here: bool,
    out: &mut Vec<Placed>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for &i in children.get(&parent).map(Vec::as_slice).unwrap_or_default() {
        let n = &nodes[i];
        // Kökte sabitli öğe en üstteki blokta zaten var.
        if skip_pinned_here && depth == 0 && !n.folder && n.pin.is_some() {
            continue;
        }
        out.push(Placed {
            ix: i,
            depth,
            lifted: false,
        });
        if n.folder && is_open.is_some_and(|f| f(&n.id)) {
            walk(nodes, children, Some(&n.id), depth + 1, is_open, false, out);
        }
    }
}

/// `folder`ın altındaki (iç içe dahil) en önde gelen öğenin anahtarı.
fn best_below(
    nodes: &[Node],
    children: &HashMap<Option<&str>, Vec<usize>>,
    keys: &[Key],
    folder: &str,
    depth: usize,
) -> Option<Key> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut best: Option<Key> = None;
    for &i in children
        .get(&Some(folder))
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let k = if nodes[i].folder {
            best_below(nodes, children, keys, &nodes[i].id, depth + 1)
        } else {
            Some(keys[i].clone())
        };
        if let Some(k) = k {
            if best.as_ref().is_none_or(|b| k < *b) {
                best = Some(k);
            }
        }
    }
    best
}

/// Her klasörün altındaki öğe sayısı (iç içe klasörler dahil, klasörün
/// kendisi sayılmaz).
pub fn item_counts(nodes: &[Node]) -> HashMap<String, usize> {
    let folder_ids: HashSet<&str> = nodes
        .iter()
        .filter(|n| n.folder)
        .map(|n| n.id.as_str())
        .collect();
    let mut direct: HashMap<&str, usize> = HashMap::new();
    let mut subfolders: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in nodes {
        let Some(p) = n
            .parent
            .as_deref()
            .filter(|p| folder_ids.contains(p) && *p != n.id)
        else {
            continue;
        };
        if n.folder {
            subfolders.entry(p).or_default().push(&n.id);
        } else {
            *direct.entry(p).or_default() += 1;
        }
    }
    fn total(
        f: &str,
        direct: &HashMap<&str, usize>,
        sub: &HashMap<&str, Vec<&str>>,
        depth: usize,
    ) -> usize {
        if depth > MAX_DEPTH {
            return 0;
        }
        direct.get(f).copied().unwrap_or(0)
            + sub
                .get(f)
                .map(|v| v.iter().map(|s| total(s, direct, sub, depth + 1)).sum())
                .unwrap_or(0)
    }
    folder_ids
        .iter()
        .map(|f| (f.to_string(), total(f, &direct, &subfolders, 0)))
        .collect()
}

/// Klasörün (üstündeki klasörlerle birlikte) kök yolu: kökten başlayıp
/// klasörün kendisinde biter. Bilinmeyen klasör için boş.
pub fn path_to<'a>(nodes: &'a [Node], folder: &str) -> Vec<&'a Node> {
    let by_id: HashMap<&str, &Node> = nodes
        .iter()
        .filter(|n| n.folder)
        .map(|n| (n.id.as_str(), n))
        .collect();
    let mut path = Vec::new();
    let mut cur = by_id.get(folder).copied();
    while let Some(n) = cur {
        if path.len() > MAX_DEPTH {
            break;
        }
        path.push(n);
        cur = n
            .parent
            .as_deref()
            .and_then(|p| by_id.get(p).copied())
            .filter(|p| p.id != n.id);
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        id: &str,
        parent: Option<&str>,
        folder: bool,
        num: i64,
        pin: Option<usize>,
        seq: usize,
    ) -> Node {
        Node {
            id: id.into(),
            parent: parent.map(String::from),
            folder,
            key: Key::new(0, num, ""),
            pin,
            seq,
        }
    }

    fn ids(nodes: &[Node], placed: &[Placed]) -> Vec<String> {
        placed
            .iter()
            .map(|p| {
                format!(
                    "{}{}{}",
                    "-".repeat(p.depth),
                    if p.lifted { "^" } else { "" },
                    nodes[p.ix].id
                )
            })
            .collect()
    }

    /// kök: a, [F: b, [G: c], d], e
    fn sample() -> Vec<Node> {
        vec![
            node("a", None, false, 5, None, 0),
            node("F", None, true, 0, None, 1),
            node("b", Some("F"), false, 4, None, 2),
            node("G", Some("F"), true, 0, None, 3),
            node("c", Some("G"), false, 1, None, 4),
            node("d", Some("F"), false, 2, None, 5),
            node("e", None, false, 3, None, 6),
        ]
    }

    fn opts<'a>(is_open: Option<&'a dyn Fn(&str) -> bool>) -> Options<'a> {
        Options {
            root: None,
            is_open,
            lift_pins: true,
            folders_by_children: true,
        }
    }

    #[test]
    fn closed_folders_show_only_the_root_level_ordered_by_key() {
        let nodes = sample();
        let placed = layout(&nodes, &opts(None));
        // F'nin anahtarı içindekilerin en küçüğü (c = 1); sıra: F(1), e(3), a(5).
        assert_eq!(ids(&nodes, &placed), ["F", "e", "a"]);
    }

    #[test]
    fn open_folders_nest_their_contents_in_place() {
        let nodes = sample();
        let open = |f: &str| f == "F" || f == "G";
        let placed = layout(&nodes, &opts(Some(&open)));
        // F içinde: G(c=1), d(2), b(4); G içinde: c.
        assert_eq!(
            ids(&nodes, &placed),
            ["F", "-G", "--c", "-d", "-b", "e", "a"]
        );
        let only_f = |f: &str| f == "F";
        let placed = layout(&nodes, &opts(Some(&only_f)));
        assert_eq!(ids(&nodes, &placed), ["F", "-G", "-d", "-b", "e", "a"]);
    }

    #[test]
    fn entering_a_folder_shows_its_direct_children_only() {
        let nodes = sample();
        let placed = layout(
            &nodes,
            &Options {
                root: Some("F"),
                is_open: None,
                lift_pins: true,
                folders_by_children: true,
            },
        );
        assert_eq!(ids(&nodes, &placed), ["G", "d", "b"]);
        let placed = layout(
            &nodes,
            &Options {
                root: Some("G"),
                is_open: None,
                lift_pins: true,
                folders_by_children: true,
            },
        );
        assert_eq!(ids(&nodes, &placed), ["c"]);
        // Bilinmeyen klasör: kök gösterilir (bayat durum).
        let placed = layout(
            &nodes,
            &Options {
                root: Some("yok"),
                is_open: None,
                lift_pins: true,
                folders_by_children: true,
            },
        );
        assert_eq!(ids(&nodes, &placed), ["F", "e", "a"]);
    }

    #[test]
    fn pinned_items_are_lifted_at_the_root_and_first_among_their_siblings() {
        let mut nodes = sample();
        nodes[0].pin = Some(1); // a (kökte)
        nodes[4].pin = Some(0); // c (G'nin içinde)
        let open = |f: &str| f == "F" || f == "G";
        let placed = layout(&nodes, &opts(Some(&open)));
        // Üstte sabitler (sabitlenme sırasıyla), sonra ağaç; a ağaçta yinelenmez,
        // c yerinde de durur.
        assert_eq!(
            ids(&nodes, &placed),
            ["^c", "^a", "F", "-G", "--c", "-d", "-b", "e"]
        );
        // Klasöre girilince blok yok; sabitli öğe kardeşlerinin önünde.
        nodes[5].pin = Some(2); // d, F içinde
        let placed = layout(
            &nodes,
            &Options {
                root: Some("F"),
                is_open: None,
                lift_pins: true,
                folders_by_children: true,
            },
        );
        assert_eq!(ids(&nodes, &placed), ["d", "G", "b"]);
    }

    #[test]
    fn folder_keys_can_be_their_own_and_empty_folders_go_last() {
        let mut nodes = sample();
        nodes.push(node("H", None, true, 0, None, 7));
        let placed = layout(&nodes, &opts(None));
        assert_eq!(
            ids(&nodes, &placed),
            ["F", "e", "a", "H"],
            "boş klasör sona"
        );
        // Kendi anahtarı: F'nin num'u 0, H'nin 0, e'nin 3: klasörler önde, seq sırası.
        let placed = layout(
            &nodes,
            &Options {
                root: None,
                is_open: None,
                lift_pins: true,
                folders_by_children: false,
            },
        );
        assert_eq!(ids(&nodes, &placed), ["F", "H", "e", "a"]);
    }

    #[test]
    fn orphans_and_cycles_do_not_hang_or_lose_items() {
        // Üst klasörü olmayan liste kökte görünür.
        let nodes = vec![
            node("x", Some("kayıp"), false, 1, None, 0),
            node("y", None, false, 2, None, 1),
        ];
        assert_eq!(ids(&nodes, &layout(&nodes, &opts(None))), ["x", "y"]);
        // Döngü: A içinde B, B içinde A. Köke bağlı olmadıkları için görünmezler
        // ama sonsuz döngü olmaz; öğeleri kök gibi davranan bir listede kalır.
        let nodes = vec![
            node("A", Some("B"), true, 0, None, 0),
            node("B", Some("A"), true, 0, None, 1),
            node("z", None, false, 1, None, 2),
        ];
        let open = |_: &str| true;
        assert_eq!(ids(&nodes, &layout(&nodes, &opts(Some(&open)))), ["z"]);
        assert_eq!(item_counts(&nodes).len(), 2);
        assert!(path_to(&nodes, "A").len() <= MAX_DEPTH + 2);
        // Kendi kendinin klasörü olan düğüm kökte sayılır.
        let nodes = vec![node("S", Some("S"), true, 0, None, 0)];
        assert_eq!(ids(&nodes, &layout(&nodes, &opts(None))), ["S"]);
    }

    #[test]
    fn counts_include_nested_folders_and_paths_run_from_the_root() {
        let nodes = sample();
        let counts = item_counts(&nodes);
        assert_eq!((counts["F"], counts["G"]), (3, 1), "F: b, d ve G'deki c");
        let path: Vec<&str> = path_to(&nodes, "G").iter().map(|n| n.id.as_str()).collect();
        assert_eq!(path, ["F", "G"]);
        assert!(path_to(&nodes, "yok").is_empty());
    }

    #[test]
    fn ties_keep_the_library_order() {
        let nodes = vec![
            node("p", None, false, 1, None, 2),
            node("q", None, false, 1, None, 0),
            node("r", None, false, 1, None, 1),
        ];
        assert_eq!(ids(&nodes, &layout(&nodes, &opts(None))), ["q", "r", "p"]);
    }
}
