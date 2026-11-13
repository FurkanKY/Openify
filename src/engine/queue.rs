//! Çalma sırası modeli: bağlam (albüm / liste / sanatçı şarkıları) + kullanıcı
//! sırası ("sıraya ekle"), karıştırma, tekrar, geçmiş.
//!
//! Saf mantık: librespot, ağ, disk yok; her davranış birim testle sınanır.
//! Motor (`mod.rs`) bu modelin döndürdüğü şarkı adresini çaların `load`'una
//! verir ve modelin görünümünü arayüze gönderir.
//!
//! Model (hepsi bu yapıda):
//!
//!   geçmiş ── şimdi çalan ── kullanıcı sırası ── bağlamın kalanı
//!   `history`   `current`        `user`            `order[cursor..]`
//!
//! - `ctx`: bağlamın şarkıları, bağlamın kendi sırasıyla; hiç değişmez.
//! - `order`: bağlam indekslerinin çalma sırası. Karıştırma kapalıyken
//!   0..n (kimlik), açıkken dengeli permütasyon (aynı sanatçı art arda
//!   gelmez, kaçınılabildikçe; bkz. `balanced_shuffle`). `order[cursor..]` bağlamın kalanı;
//!   `order[..cursor]` çalınmış kısım.
//! - `user`: "sıraya ekle" şarkıları. Bağlamdan ÖNCE çalar, bitince bağlamda
//!   kalınan yerden sürülür (Spotify gibi).
//! - `history`: önceden çalınanlar (Önceki bunlardan döner), en çok 100.

use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};

use super::{
    hidden::HiddenList,
    library::{CollectionId, TrackRow},
    radio::RadioInfo,
    rules::{self, Rules, Traits},
};

/// Geçmişte tutulan en çok şarkı.
pub const HISTORY_LIMIT: usize = 100;
/// Kullanıcı sırasının en çok uzunluğu.
pub const USER_QUEUE_LIMIT: usize = 500;
/// Görünümde (arayüze giden olayda) gösterilen bağlam şarkısı sayısı.
pub const VIEW_UPCOMING: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    /// Bağlam bitince çalma durur.
    #[default]
    Off,
    /// Bağlam bitince başa dönülür.
    All,
    /// Şarkı bitince aynı şarkı yeniden çalar (Sonraki yine ilerler).
    One,
}

/// Sıradaki bir şarkının kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueSource {
    /// Kullanıcı ekledi ("sıraya ekle" / "sonra çal").
    User,
    /// Çalan bağlamdan (albüm, liste, sanatçı).
    Context,
}

/// Arayüzde gösterilecek bir sıra satırı. `track` yoksa (bilgisi henüz
/// önbellekte bulunamadı) yalnız adres vardır; motor bulunca görünümü yeniler.
#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub uri: String,
    pub track: Option<TrackRow>,
    pub source: QueueSource,
}

/// Çalan şarkı ve sıradakiler.
#[derive(Debug, Clone, Default)]
pub struct QueueView {
    pub current: Option<QueueEntry>,
    /// Önce kullanıcı sırası (`user_len` tane), sonra bağlamın kalanı (en çok
    /// `VIEW_UPCOMING`; tekrar "hepsi"yse sıradaki tur da eklenir).
    pub upcoming: Vec<QueueEntry>,
    pub user_len: usize,
    /// Bağlamda çalınmayı bekleyen toplam şarkı (görünen pencere dahil).
    pub context_remaining: usize,
    pub context: Option<CollectionId>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Çalan bağlam bir radyoysa kimliği (başlık için); sıra sonsuzdur:
    /// `context_remaining` sona yaklaşınca motor devamını getirir.
    pub radio: Option<RadioInfo>,
}

/// Kullanıcı sırasındaki bir şarkı. Bilgisi varsa (arayüz verdi) taşınır.
#[derive(Debug, Clone)]
pub struct UserItem {
    pub uri: String,
    pub track: Option<TrackRow>,
}

/// Şarkının sıraya nereden girdiği: geri sarınca (Önceki) nereye
/// döneceğini belirler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// Kullanıcı sırasından (ya da başka bir bağlamdan kalma geçmiş).
    User,
    /// Bağlamın `ctx` listesindeki bu indeksten.
    Context(usize),
}

#[derive(Debug, Clone)]
pub(super) struct Played {
    pub uri: String,
    pub origin: Origin,
    pub track: Option<TrackRow>,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    pub(super) context: Option<CollectionId>,
    pub(super) ctx: Vec<String>,
    /// `ctx` ile aynı indeksli şarkı bilgileri (varsa).
    pub(super) rows: Option<Arc<Vec<TrackRow>>>,
    /// `ctx` ile aynı indeksli ana sanatçı kimliği (dengeli karıştırma için):
    /// aynı sayı aynı sanatçı, 0 bilinmiyor. Boşsa hepsi bilinmiyor (eski
    /// kayıt ya da bilgisiz bağlam). Bkz. `artist_groups`.
    pub(super) artists: Vec<u32>,
    pub(super) order: Vec<usize>,
    pub(super) cursor: usize,
    pub(super) current: Option<Played>,
    pub(super) user: VecDeque<UserItem>,
    pub(super) history: VecDeque<Played>,
    /// Bağlamdan en son çalan şarkının indeksi (şimdi çalan kullanıcı
    /// şarkısı olsa bile).
    pub(super) last_ctx: Option<usize>,
    pub(super) shuffle: bool,
    pub(super) repeat: RepeatMode,
    pub(super) seed: u64,
    pub(super) lap: u32,
    /// `ctx` ya da `order` her değiştiğinde artar (kaldığı yerden sürme
    /// kaydı büyük listeyi yalnız bu değişince yeniden yazar).
    pub(super) order_gen: u64,
    /// Karıştırma ve sıra kuralları (bkz. `rules.rs`): bağlamın çalma
    /// sırasına (`order`) girecek şarkıları süzer. Bağlam değişse de kalır.
    pub(super) rules: Rules,
    /// `ctx` ile aynı indeksli şarkı özellikleri (açık söz, popülerlik);
    /// boşsa bilinmiyor.
    pub(super) traits: Vec<Traits>,
    /// Yerel olarak gizlenen şarkılar (bkz. `hidden.rs`): çalma sırasına
    /// girmezler (genel ya da bu bağlama özgü); kullanıcının elle seçtiği
    /// şarkıya (`restart_with`'in `index`'i, sıraya eklenenler) dokunulmaz.
    /// Bağlam değişse de kalır.
    pub(super) hidden: Arc<HiddenList>,
    /// Çalan bağlam bir radyo mu (bkz. `radio.rs`)? Yeni bağlam başlayınca düşer.
    pub(super) radio: Option<RadioInfo>,
    /// Kurallar için `ctx` ile aynı indeksli satırlar, `rows` yokken
    /// (kaldığı yerden sürme, sanatçı bağlamı): yalnız adlara bakılır, ekranda
    /// gösterilmez. Bilinmeyen şarkı için boş ad/sanatçılı satır.
    pub(super) info: Option<Arc<Vec<TrackRow>>>,
}

// ── Rastgelelik ─────────────────────────────────────────────────────────

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `0..n` aralığında (n > 0) sapmasız sayılacak kadar düzgün sayı.
fn bounded(state: &mut u64, n: usize) -> usize {
    ((splitmix64(state) as u128 * n as u128) >> 64) as usize
}

/// Fisher-Yates karıştırması: aynı tohum, aynı girdi → aynı permütasyon.
pub fn fisher_yates<T>(items: &mut [T], seed: u64) {
    let mut state = seed;
    for i in (1..items.len()).rev() {
        let j = bounded(&mut state, i + 1);
        items.swap(i, j);
    }
}

fn lap_seed(seed: u64, lap: u32) -> u64 {
    seed ^ (lap as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)
}

/// `[0, 1)` aralığında düzgün sayı.
fn unit(state: &mut u64) -> f64 {
    (splitmix64(state) >> 11) as f64 / (1u64 << 53) as f64
}

// ── Dengeli karıştırma ──────────────────────────────────────────────────
//
// Düz karıştırmada aynı sanatçının şarkıları sık sık art arda gelir (30
// şarkılık listede 5 şarkılık sanatçı için bu olasılık ~%50). Dengeli
// karıştırma iki adımda:
//
// 1. Yayma: her sanatçının şarkıları kendi içinde karışır ve [0, 1)
//    ekseninde eşit aralıkla dizilir (k şarkı → 1/k arayla), rastgele bir
//    başlangıç kaymasıyla (0..1/k) ve her şarkıya aralığın ±%10'u kadar
//    titreşimle. Tek şarkılı ve sanatçısı bilinmeyen şarkılar (bunlar için
//    düz karıştırma) kendi aralarında karışıp eksene tabakalı dağılır. Hepsi
//    konuma göre sıralanır.
// 2. Onarım: sıra baştan gezilir; her adımda yayılmış sıradaki ilk uygun
//    şarkı alınır. Uygun: bir öncekiyle aynı sanatçı değil VE geri kalanı
//    hâlâ art arda gelmeden dizilebiliyor (sayım koşulu, aşağıda) VE
//    (yakında bulunabiliyorsa) aynı sanatçının bir önceki şarkısından ideal
//    aralığın en az yarısı kadar uzakta. Bir sanatçı listenin yarısından
//    fazlasıysa tekrar kaçınılmaz: onun şarkıları, araya girecek şarkı
//    sayısı + 1 eşit öbeğe bölünüp öbekler birer şarkı gibi dizilir
//    (kaçınılmaz tekrarlar eşit dağılır, başka hiçbir şey art arda gelmez).
//
// Sayım koşulu: R şarkı, ilki P sanatçısı olamaz (bir önceki çalan P). Art
// arda aynı sanatçı gelmeden dizilebilir ⇔ P'nin sayısı ≤ ⌊R/2⌋ ve her
// başka sanatçının sayısı ≤ ⌈R/2⌉. (Gerekli: R yerin ancak ⌈R/2⌉'sine
// birbirine değmeden konabilir, P ilk yere konamaz. Yeterli: "en kalabalık,
// öncekinden farklı sanatçı" açgözlü seçimi koşulu korur.) Bilinmeyen
// sanatçılı şarkılar birbirinden farklı sayılır.

/// Bağlamın ana sanatçı kimlikleri: ilk sanatçının adı (büyük / küçük harf
/// ve boşluk farkı yok sayılır), görülme sırasıyla 1, 2, …; adı olmayan 0.
///
/// Neden ad: şarkı satırı (`TrackRow`) bütün kaynaklarda (kütüphane, liste,
/// albüm / sanatçı sayfası, arama, palet) sanatçı adlarını taşır; adresleri
/// ise yalnız ayrıca istenen bağlarda var (`pages::TrackLinks`, satır
/// çizilince). Ad her şarkıda var ve aynı sanatçının bütün şarkılarında aynı;
/// iki ayrı sanatçının aynı adı taşıması yalnız onları gereksiz yere ayırır.
pub(super) fn artist_groups(rows: &[TrackRow]) -> Vec<u32> {
    let mut ids: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    rows.iter()
        .map(|r| {
            let primary = r
                .artists
                .split(", ")
                .next()
                .unwrap_or("")
                .trim()
                .to_lowercase();
            if primary.is_empty() {
                return 0;
            }
            let next = ids.len() as u32 + 1;
            *ids.entry(primary).or_insert(next)
        })
        .collect()
}

/// Kalan sayımlar: sanatçı başına şarkı sayısı, sayı başına sanatçı sayısı
/// ve en büyük sayı (sayılar birer azaldığı için O(1) güncellenir).
struct Counts {
    of: Vec<usize>,
    num_at: Vec<usize>,
    max: usize,
}

impl Counts {
    fn new(artists: impl Iterator<Item = u32>, groups: usize) -> Self {
        let mut of = vec![0usize; groups + 1];
        for a in artists.filter(|a| *a > 0) {
            of[a as usize] += 1;
        }
        let max = of.iter().copied().max().unwrap_or(0);
        let mut num_at = vec![0usize; max + 1];
        for c in of.iter().skip(1) {
            num_at[*c] += 1;
        }
        Self { of, num_at, max }
    }

    fn of(&self, a: u32) -> usize {
        if a == 0 {
            0
        } else {
            self.of.get(a as usize).copied().unwrap_or(0)
        }
    }

    /// `a` dışındaki sanatçıların en büyük sayısı.
    fn max_excluding(&self, a: u32) -> usize {
        if a == 0 || self.of(a) != self.max || self.num_at[self.max] > 1 {
            return self.max;
        }
        (1..self.max)
            .rev()
            .find(|c| self.num_at[*c] > 0)
            .unwrap_or(0)
    }

    fn take(&mut self, a: u32) {
        if a == 0 {
            return;
        }
        let c = self.of[a as usize];
        self.num_at[c] -= 1;
        self.of[a as usize] = c - 1;
        self.num_at[c - 1] += 1;
        while self.max > 0 && self.num_at[self.max] == 0 {
            self.max -= 1;
        }
    }

    /// `remaining` şarkı, bir önceki `prev` sanatçısı: art arda gelmeden
    /// dizilebilir mi? (Sayım koşulu, yukarıda.)
    fn feasible(&self, remaining: usize, prev: u32) -> bool {
        self.of(prev) <= remaining / 2 && self.max_excluding(prev) <= remaining.div_ceil(2)
    }

    /// `a` sanatçılı şarkı şimdi alınırsa (bir önceki `prev`) geri kalanı
    /// dizilebilir mi?
    fn can_take(&self, a: u32, prev: u32, remaining: usize) -> bool {
        if a != 0 && a == prev {
            return false;
        }
        let after = remaining - 1;
        let own = self.of(a).saturating_sub(1);
        // `a` alınınca kendi sayısı bir azalır; ötekiler aynı kalır.
        let others = self.max_excluding(a);
        (a == 0 || own <= after / 2) && others <= after.div_ceil(2)
    }
}

/// Bağlam indekslerini (`items`) dengeli karıştırır (bkz. yukarı). `artists`:
/// bağlam indeksine göre sanatçı kimliği (boşsa hepsi bilinmiyor); `prev`:
/// hemen önce çalan şarkının sanatçısı (0 bilinmiyor). Aynı tohum, aynı
/// girdi → aynı sıra. Sanatçısı bilinen hiç şarkı yoksa düz Fisher-Yates.
pub fn balanced_shuffle(items: &mut [usize], artists: &[u32], prev: u32, seed: u64) {
    let artist = |i: usize| artists.get(i).copied().unwrap_or(0);
    let groups = items.iter().map(|i| artist(*i)).max().unwrap_or(0) as usize;
    if groups == 0 {
        fisher_yates(items, seed);
        return;
    }
    let mut state = seed;

    // 1. Yayma.
    let mut by_artist: Vec<Vec<usize>> = vec![Vec::new(); groups + 1];
    for &i in items.iter() {
        by_artist[artist(i) as usize].push(i);
    }
    let mut keyed: Vec<(f64, usize)> = Vec::with_capacity(items.len());
    // Tek şarkılı ve sanatçısı bilinmeyen şarkılar birlikte karışır ve eksene
    // tabakalı dağılır (her biri kendi 1/L'lik diliminde rastgele bir yerde):
    // tek tek düzgün rastgele konsalar yığılıp seyrekleşerek kalabalık
    // sanatçıların aralığını oynatırlardı (120 şarkıda en geniş aralık 10
    // yerine 20+).
    let mut loose: Vec<usize> = by_artist[0].clone();
    for group in by_artist.iter_mut().skip(1).filter(|g| g.len() == 1) {
        loose.push(group.pop().expect("tek şarkı"));
    }
    fisher_yates(&mut loose, splitmix64(&mut state));
    let slice = 1.0 / loose.len().max(1) as f64;
    for (r, &i) in loose.iter().enumerate() {
        keyed.push(((r as f64 + unit(&mut state)) * slice, i));
    }
    for group in by_artist.iter_mut().skip(1).filter(|g| !g.is_empty()) {
        fisher_yates(group, splitmix64(&mut state));
        let step = 1.0 / group.len() as f64;
        let offset = unit(&mut state) * step;
        for (j, &i) in group.iter().enumerate() {
            let jitter = (unit(&mut state) - 0.5) * 0.2 * step;
            keyed.push((offset + j as f64 * step + jitter, i));
        }
    }
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let spread: Vec<usize> = keyed.into_iter().map(|(_, i)| i).collect();

    // 2. Birimler. Normalde her şarkı bir birim. Bir sanatçı art arda
    // gelmeden dizilemeyecek kadar kalabalıksa (yarıdan fazla), onun şarkıları
    // araya girecek şarkı sayısı + 1 öbeğe eşit bölünür ve her öbek bir birim
    // olur: kaçınılmaz tekrarlar eşit dağılır, öbekler arasında hep başka bir
    // sanatçı vardır, başka hiçbir sanatçı art arda gelmez.
    let n = spread.len();
    let all = Counts::new(spread.iter().map(|i| artist(*i)), groups);
    // (sanatçı, `flat` içinde başlangıç, uzunluk)
    let mut units: Vec<(u32, usize, usize)> = Vec::with_capacity(n);
    let mut flat: Vec<usize> = Vec::with_capacity(n);
    if all.feasible(n, prev) {
        flat.extend_from_slice(&spread);
        units.extend(spread.iter().enumerate().map(|(k, i)| (artist(*i), k, 1)));
    } else {
        let dom = (1..=groups as u32)
            .max_by_key(|a| (all.of(*a), std::cmp::Reverse(*a)))
            .unwrap_or(0);
        let c = all.of(dom);
        let others = n - c;
        // Hemen önce çalan da bu sanatçıysa o, ilk öbeğin başı sayılır (öbek
        // boyları onunla birlikte eşit olur; ilk öbek ona bitişik çalar).
        let lead = usize::from(prev == dom);
        let total = c + lead;
        let k = (others + 1).clamp(1, total);
        let dom_items: Vec<usize> = spread
            .iter()
            .copied()
            .filter(|i| artist(*i) == dom)
            .collect();
        // j. öbeğin sınırları (öncekiyle birlikte sayılarak): büyük öbekler de
        // eşit aralıkla dağılır.
        let bound = |j: usize| (j * total / k).saturating_sub(lead);
        let mut seen_dom = 0;
        let mut chunk = 0;
        for &i in &spread {
            if artist(i) != dom {
                units.push((artist(i), flat.len(), 1));
                flat.push(i);
                continue;
            }
            while bound(chunk + 1) <= seen_dom {
                chunk += 1; // boş öbek (yalnız öncekinden ibaret ilk öbek)
            }
            if seen_dom == bound(chunk) {
                let len = bound(chunk + 1) - bound(chunk);
                units.push((dom, flat.len(), len));
                flat.extend_from_slice(&dom_items[seen_dom..seen_dom + len]);
                chunk += 1;
            }
            seen_dom += 1;
        }
    }

    // 3. Onarım (birimler üzerinde). İlk öbek öncekinin devamıysa (yukarı)
    // kısıt kalkar: o öbeğin önde gelmesini sayım koşulu kendisi zorlar.
    let m = units.len();
    let mut counts = Counts::new(units.iter().map(|u| u.0), groups);
    let prev = if counts.feasible(m, prev) { prev } else { 0 };
    if !counts.feasible(m, prev) {
        items.copy_from_slice(&spread);
        return;
    }
    // Yumuşak ayrım: aynı sanatçının iki birimi arasında, ideal aralığın
    // (birim sayısı / sanatçının birimi) yarısından azı kalmasın; ileride
    // (en çok `LOOKAHEAD` birim) buna uyan yoksa yalnız katı kurala uyulur.
    // Düz rastgele yerleşimin "iki şarkı arayla yine aynı sanatçı" anlarını
    // keser.
    const LOOKAHEAD: usize = 24;
    let soft: Vec<usize> = (0..=groups)
        .map(|a| match counts.of(a as u32) {
            0 => 0,
            t => (m / t).div_ceil(2),
        })
        .collect();
    // Sanatçının son biriminin yeri + 1 (0: hemen önce çalan `prev`).
    let mut last: Vec<Option<usize>> = vec![None; groups + 1];
    if let Some(l) = last.get_mut(prev as usize).filter(|_| prev != 0) {
        *l = Some(0);
    }
    let mut taken = vec![false; m];
    let mut head = 0;
    let mut prev = prev;
    let mut write = 0;
    for out in 0..m {
        while taken[head] {
            head += 1;
        }
        let remaining = m - out;
        let mut hard = None;
        let mut pick = None;
        let mut seen = 0;
        for j in head..m {
            if taken[j] {
                continue;
            }
            seen += 1;
            let a = units[j].0;
            if counts.can_take(a, prev, remaining) {
                hard.get_or_insert(j);
                let gap_ok =
                    a == 0 || last[a as usize].is_none_or(|l| out + 1 - l >= soft[a as usize]);
                if gap_ok {
                    pick = Some(j);
                    break;
                }
            }
            if seen >= LOOKAHEAD && hard.is_some() {
                break;
            }
        }
        // Koşul korunduğu için `hard` hep bulunur; yine de güvenli.
        let pick = pick.or(hard).unwrap_or(head);
        taken[pick] = true;
        let (a, start, len) = units[pick];
        counts.take(a);
        if a != 0 {
            last[a as usize] = Some(out + 1);
        }
        prev = a;
        items[write..write + len].copy_from_slice(&flat[start..start + len]);
        write += len;
    }
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn bump(&mut self) {
        self.order_gen = self.order_gen.wrapping_add(1);
    }

    /// Sırada (çalan, sıradaki, geçmiş) hiçbir şey yok mu? (Testler; arayüz
    /// durumu `QueueView`'dan okur.)
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.current.is_none() && self.user.is_empty() && self.ctx.is_empty()
    }

    #[cfg(test)]
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    #[cfg(test)]
    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn context(&self) -> Option<&CollectionId> {
        self.context.as_ref()
    }

    pub fn order_gen(&self) -> u64 {
        self.order_gen
    }

    #[cfg(test)]
    pub fn user_len(&self) -> usize {
        self.user.len()
    }

    pub fn current_uri(&self) -> Option<&str> {
        self.current.as_ref().map(|p| p.uri.as_str())
    }

    /// Çalan şarkı bağlamdansa bağlamdaki (özgün sıradaki) indeksi.
    pub fn context_index(&self) -> Option<usize> {
        match self.current.as_ref()?.origin {
            Origin::Context(i) => Some(i),
            Origin::User => None,
        }
    }

    fn current_ctx(&self) -> Option<usize> {
        self.context_index()
    }

    /// Çalan şarkının bilgisi (sırada varsa).
    pub fn current_row(&self) -> Option<TrackRow> {
        let cur = self.current.as_ref()?;
        match cur.origin {
            Origin::Context(i) => self.rows.as_ref().and_then(|r| r.get(i).cloned()),
            Origin::User => cur.track.clone(),
        }
    }

    // ── Bağlam başlatma ────────────────────────────────────────────────

    /// Yeni bir bağlam başlatır: `index`'teki şarkı çalar. Karıştırma ve
    /// tekrar durumu korunur; karıştırma açıksa `index` önce çalar, kalanı
    /// karışır. Çalan şarkı geçmişe gider (Önceki ona döner). Kullanıcı sırası
    /// `keep_user` ise korunur. Çalacak şarkının adresini döner.
    #[cfg(test)]
    pub fn restart(
        &mut self,
        context: Option<CollectionId>,
        uris: Vec<String>,
        rows: Option<Arc<Vec<TrackRow>>>,
        index: usize,
        seed: u64,
        keep_user: bool,
    ) -> Option<String> {
        self.restart_with(
            context,
            uris,
            rows,
            index,
            seed,
            keep_user,
            None,
            Vec::new(),
        )
    }

    /// `restart` + kurallar için bilgi: `info` (satırlar, `rows` yoksa) ve
    /// `traits` (açık söz / popülerlik; bilinmiyorsa boş) çalma sırası
    /// kurulmadan verilir, sıra bir kez, kurallara uyarak kurulur. `index`'teki
    /// şarkı kurallara bakılmadan çalar (kullanıcı onu seçti).
    #[allow(clippy::too_many_arguments)]
    pub fn restart_with(
        &mut self,
        context: Option<CollectionId>,
        uris: Vec<String>,
        rows: Option<Arc<Vec<TrackRow>>>,
        index: usize,
        seed: u64,
        keep_user: bool,
        info: Option<Arc<Vec<TrackRow>>>,
        traits: Vec<Traits>,
    ) -> Option<String> {
        if index >= uris.len() {
            return None;
        }
        if let Some(cur) = self.current.take() {
            let track = cur.track.or_else(|| self.row_of(&cur.origin));
            self.push_history(Played {
                uri: cur.uri,
                origin: Origin::User,
                track,
            });
        }
        // Eski bağlamın indeksleri yeni bağlamda anlamsız.
        for h in self.history.iter_mut() {
            h.track = h.track.take().or_else(|| match h.origin {
                Origin::Context(i) => self.rows.as_ref().and_then(|r| r.get(i).cloned()),
                Origin::User => None,
            });
            h.origin = Origin::User;
        }
        if !keep_user {
            self.user.clear();
        }
        self.context = context;
        self.radio = None;
        let info = info.filter(|i| i.len() == uris.len());
        self.artists = rows
            .as_deref()
            .or(info.as_deref())
            .filter(|r| r.len() == uris.len())
            .map(|r| artist_groups(r))
            .unwrap_or_default();
        self.traits = if traits.len() == uris.len() {
            traits
        } else {
            Vec::new()
        };
        self.info = info;
        self.ctx = uris;
        self.rows = rows;
        self.seed = seed;
        self.lap = 0;
        self.last_ctx = Some(index);
        let uri = self.ctx[index].clone();
        self.current = Some(Played {
            uri: uri.clone(),
            origin: Origin::Context(index),
            track: None,
        });
        self.rebuild_order(Some(index));
        Some(uri)
    }

    /// Bağlamdaki `i`. şarkının sanatçı kimliği (0 bilinmiyor).
    fn artist_of(&self, i: usize) -> u32 {
        self.artists.get(i).copied().unwrap_or(0)
    }

    fn row_of(&self, origin: &Origin) -> Option<TrackRow> {
        match origin {
            Origin::Context(i) => self.rows.as_ref().and_then(|r| r.get(*i).cloned()),
            Origin::User => None,
        }
    }

    /// Kuralların (bkz. `rules.rs`) çalma havuzuna bıraktığı `candidates`
    /// indeksleri (sırası korunur). `heard`: bu turda çalınmışlar; `dedupe`:
    /// aynı şarkının sürümleri elensin mi (yalnız karıştırılmış sırada).
    /// Kural yoksa ya da şarkı satırları (adlar) bilinmiyorsa hepsi.
    fn pool(&self, candidates: &[usize], heard: &[usize], dedupe: bool) -> Vec<usize> {
        let n = self.ctx.len();
        // Gizlenen şarkılar (adrese göre; satır bilgisi gerekmez) en başta düşer.
        let visible: Vec<usize>;
        let candidates = if self.hidden.filters_nothing() {
            candidates
        } else {
            let key = self.context.as_ref().map(CollectionId::key);
            visible = candidates
                .iter()
                .copied()
                .filter(|&i| self.ctx.get(i).is_none_or(|u| !self.hidden.hides(key, u)))
                .collect();
            &visible
        };
        let rows = self
            .rows
            .as_deref()
            .or(self.info.as_deref())
            .filter(|r| r.len() == n);
        match rows {
            Some(rows) if !self.rules.is_empty() => {
                let traits: &[Traits] = if self.traits.len() == n {
                    &self.traits
                } else {
                    &[]
                };
                rules::select(rows, traits, &self.rules, dedupe, heard, candidates)
            }
            _ => candidates.to_vec(),
        }
    }

    /// `order` ve `cursor`'ı geçerli karıştırma durumuna göre kurar. `first`
    /// (çalan bağlam şarkısı) çalınmış sayılır; kalanı (geçmişte çalınanlar
    /// hariç) karışır. Kurallar çalma havuzunu süzer; `first`'e (kullanıcının
    /// seçtiği şarkıya) dokunmaz.
    fn rebuild_order(&mut self, first: Option<usize>) {
        let n = self.ctx.len();
        let first = first.filter(|f| *f < n);
        if self.shuffle {
            let mut played: Vec<bool> = vec![false; n];
            if let Some(f) = first {
                played[f] = true;
            }
            for h in &self.history {
                if let Origin::Context(i) = h.origin
                    && i < n
                {
                    played[i] = true;
                }
            }
            let rest: Vec<usize> = (0..n).filter(|i| !played[*i]).collect();
            let heard: Vec<usize> = (0..n).filter(|i| played[*i]).collect();
            let mut rest = self.pool(&rest, &heard, true);
            let prev = first.map_or(0, |f| self.artist_of(f));
            balanced_shuffle(
                &mut rest,
                &self.artists,
                prev,
                lap_seed(self.seed, self.lap),
            );
            self.order.clear();
            if let Some(f) = first {
                self.order.push(f);
            }
            self.cursor = self.order.len();
            self.order.extend(rest);
        } else {
            // Bağlamın kendi sırası; çalan şarkıdan önceki ve sonraki şarkılar
            // kurallardan geçer (Önceki de kara listeye düşeni çalmasın).
            let split = first.map_or(0, |f| f + 1);
            let before: Vec<usize> = (0..split.saturating_sub(1)).collect();
            let after: Vec<usize> = (split..n).collect();
            let mut order = self.pool(&before, &[], false);
            order.extend(first);
            self.cursor = order.len();
            order.extend(self.pool(&after, &[], false));
            self.order = order;
        }
        self.bump();
    }

    // ── Kurallar ───────────────────────────────────────────────────────

    /// Kuralları değiştirir; bağlamın kalanı (çalınmış kısım ve çalan şarkı
    /// yerinde) yeni kurallara göre yeniden kurulur. Değişti mi?
    pub fn set_rules(&mut self, rules: Rules) -> bool {
        if rules == self.rules {
            return false;
        }
        self.rules = rules;
        self.reapply_rules();
        true
    }

    /// Bağlamın şarkı özellikleri (açık söz, popülerlik) bulundu: `ctx`
    /// indeksine göre. Uzunluğu uymayan yok sayılır. Kurallar bunlara
    /// bakıyorsa kalan sıra yeniden kurulur.
    pub fn set_traits(&mut self, traits: Vec<Traits>) {
        if traits.len() != self.ctx.len() || traits == self.traits {
            return;
        }
        self.traits = traits;
        if self.rules.needs_traits() {
            self.reapply_if_needed();
        }
    }

    /// Bilinen özelliklerden `found`'da olanları (adrese göre) işler. Değişen
    /// bir şey olduysa `true` (ve kurallar bunlara bakıyorsa sıra yenilenir).
    pub fn merge_traits(&mut self, found: &std::collections::HashMap<String, Traits>) -> bool {
        if self.ctx.is_empty() || found.is_empty() {
            return false;
        }
        let mut traits = if self.traits.len() == self.ctx.len() {
            self.traits.clone()
        } else {
            vec![Traits::default(); self.ctx.len()]
        };
        let mut changed = false;
        for (slot, uri) in traits.iter_mut().zip(&self.ctx) {
            if let Some(t) = found.get(uri)
                && *slot != *t
            {
                *slot = *t;
                changed = true;
            }
        }
        if changed {
            self.set_traits(traits);
        }
        changed
    }

    /// `rows` yokken (kaldığı yerden sürme, sanatçı bağlamı) kurallar için
    /// satırlar verildi. Sanatçı kimlikleri de yoksa bunlardan çıkarılır
    /// (karıştırma sanatçıları dengeler).
    pub fn attach_info(&mut self, info: Arc<Vec<TrackRow>>) {
        if self.rows.is_some() || info.len() != self.ctx.len() || self.ctx.is_empty() {
            return;
        }
        if self.artists.is_empty() {
            self.artists = artist_groups(&info);
        }
        self.info = Some(info);
        if !self.rules.is_empty() {
            self.reapply_if_needed();
        }
    }

    /// Kuralların bakacağı satırlar yok mu (varsa `attach_info` gerek)?
    pub fn needs_info(&self) -> bool {
        !self.ctx.is_empty() && self.rows.is_none() && self.info.is_none()
    }

    /// Bağlamın şarkı adresleri (motorun özellik / bilgi sorguları için).
    pub fn ctx_uris(&self) -> &[String] {
        &self.ctx
    }

    /// Bağlamın kalanında (çalma sırasıyla) `known` demeyen şarkıların
    /// adresleri, en çok `limit` tane (özelliği, yani açık söz işareti, henüz
    /// bilinmeyenleri bulmak için).
    pub fn untraited_upcoming(&self, known: &dyn Fn(&str) -> bool, limit: usize) -> Vec<String> {
        let rest = self
            .order
            .get(self.cursor.min(self.order.len())..)
            .unwrap_or(&[]);
        rest.iter()
            .filter_map(|i| self.ctx.get(*i))
            .filter(|u| !known(u))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Bağlamdan çalınabilecek (kuralların dışlamadığı) şarkı var mı?
    fn any_playable(&self) -> bool {
        let n = self.ctx.len();
        if n == 0 {
            return false;
        }
        let key = self.context.as_ref().map(CollectionId::key);
        let shown =
            |i: usize| self.hidden.filters_nothing() || !self.hidden.hides(key, &self.ctx[i]);
        let Some(rows) = self
            .rows
            .as_deref()
            .or(self.info.as_deref())
            .filter(|r| r.len() == n)
        else {
            return (0..n).any(shown);
        };
        (0..n).any(|i| {
            shown(i)
                && (self.rules.is_empty() || !self.rules.excludes(&rows[i], self.traits.get(i)))
        })
    }

    /// Kuralların bakacağı bilgi (özellik ya da satırlar) sonradan geldi:
    /// ancak kalan sırada kuralların elediği bir şey varsa
    /// sırayı yeniden kurar. Böylece kaldığı yerden sürülen karışık sıra, her
    /// açılışta gereksiz yere karışmaz.
    fn reapply_if_needed(&mut self) {
        let n = self.ctx.len();
        let cursor = self.cursor.min(self.order.len());
        let rest = &self.order[cursor..];
        let heard: Vec<usize> = if self.shuffle {
            let mut played = vec![false; n];
            for &i in &self.order[..cursor] {
                played[i] = true;
            }
            if let Some(l) = self.last_ctx.filter(|l| *l < n) {
                played[l] = true;
            }
            (0..n).filter(|i| played[*i]).collect()
        } else {
            Vec::new()
        };
        if self.pool(rest, &heard, self.shuffle).len() != rest.len() {
            self.reapply_rules();
        }
    }

    /// Kuralları bağlamın sırasına dokunmadan benimser (kaldığı yerden sürme:
    /// kayıtlı sıra yerinde kalır; satırlar gelince `attach_info` yalnız gerekirse
    /// yeniden kurar).
    pub fn adopt_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// Çalınmış kısım (`order[..cursor]`) ve çalan şarkı yerinde kalır; bağlamın
    /// kalanı kurallara göre yeniden kurulur. Karıştırma açıkken kalan yeniden
    /// karışır (aynı sanatçı art arda gelmez kuralı korunur).
    fn reapply_rules(&mut self) {
        let n = self.ctx.len();
        if n == 0 {
            return;
        }
        self.cursor = self.cursor.min(self.order.len());
        self.order.truncate(self.cursor);
        let done: Vec<usize> = self.order.clone();
        if self.shuffle {
            let mut played = vec![false; n];
            for &i in &done {
                played[i] = true;
            }
            if let Some(l) = self.last_ctx.filter(|l| *l < n) {
                played[l] = true;
            }
            let rest: Vec<usize> = (0..n).filter(|i| !played[*i]).collect();
            let heard: Vec<usize> = (0..n).filter(|i| played[*i]).collect();
            let mut rest = self.pool(&rest, &heard, true);
            let prev = self.last_ctx.map_or(0, |l| self.artist_of(l));
            balanced_shuffle(
                &mut rest,
                &self.artists,
                prev,
                lap_seed(self.seed, self.lap),
            );
            self.order.extend(rest);
        } else {
            let start = done.iter().max().map_or(0, |m| m + 1);
            let rest: Vec<usize> = (start..n).collect();
            let rest = self.pool(&rest, &[], false);
            self.order.extend(rest);
        }
        self.bump();
    }

    // ── Gizlenen şarkılar ──────────────────────────────────────────────

    /// Gizlenen şarkılar değişti: bağlamın kalanı (çalınmış kısım ve çalan
    /// şarkı yerinde) yeniden kurulur. Değişti mi?
    pub fn set_hidden(&mut self, hidden: Arc<HiddenList>) -> bool {
        if *hidden == *self.hidden {
            return false;
        }
        self.hidden = hidden;
        self.reapply_rules();
        true
    }

    /// Gizlenenleri sıraya dokunmadan benimser (kaldığı yerden sürme); ardından
    /// `refresh_if_needed` yalnız kalan sırada gizli şarkı varsa yeniden kurar.
    pub fn adopt_hidden(&mut self, hidden: Arc<HiddenList>) {
        self.hidden = hidden;
    }

    /// Kuralların / gizlenenlerin elediği bir şey kalan sırada duruyorsa sırayı
    /// yeniden kurar (kayıtlı sıra gereksiz yere karışmaz).
    pub fn refresh_if_needed(&mut self) {
        if !self.ctx.is_empty() {
            self.reapply_if_needed();
        }
    }

    // ── Radyo ──────────────────────────────────────────────────────────

    pub fn radio(&self) -> Option<&RadioInfo> {
        self.radio.as_ref()
    }

    pub fn set_radio(&mut self, radio: Option<RadioInfo>) {
        self.radio = radio;
    }

    /// Bağlamda çalınmayı bekleyen şarkı sayısı (çalan ve kullanıcı sırası hariç).
    pub fn context_remaining(&self) -> usize {
        self.order.len().saturating_sub(self.cursor)
    }

    /// Sonsuz bağlama (radyo) yeni şarkılar ekler: zaten bağlamda olanlar ve
    /// kendi içindeki tekrarlar atılır, bağlam `radio::CTX_MAX`'ı aşmaz. Yeni
    /// şarkılar çalma sırasının SONUNA kurallardan geçerek girer (karıştırma
    /// açıksa kendi aralarında dengeli karışır); çalınmış kısma, çalan şarkıya
    /// ve kullanıcı sırasına dokunulmaz. Eklenen şarkı sayısını döner (özellikler
    /// ayrıca `merge_traits` ile işlenir).
    pub fn extend_context(&mut self, rows: Vec<TrackRow>) -> usize {
        let old_n = self.ctx.len();
        let room = super::radio::CTX_MAX.saturating_sub(old_n);
        let fresh: Vec<TrackRow> = {
            // `insert` iki işi birden yapar: bağlamdakileri ve kendi içindeki tekrarları eler.
            let mut seen: std::collections::HashSet<&str> =
                self.ctx.iter().map(String::as_str).collect();
            rows.iter()
                .filter(|r| seen.insert(r.uri.as_str()))
                .take(room)
                .cloned()
                .collect()
        };
        if fresh.is_empty() {
            return 0;
        }
        self.ctx.extend(fresh.iter().map(|r| r.uri.clone()));
        match (&mut self.rows, &mut self.info) {
            (Some(rows), _) => Arc::make_mut(rows).extend(fresh.iter().cloned()),
            (None, Some(info)) => Arc::make_mut(info).extend(fresh.iter().cloned()),
            (None, None) => {}
        }
        let n = self.ctx.len();
        match self
            .rows
            .as_deref()
            .or(self.info.as_deref())
            .filter(|r| r.len() == n)
        {
            Some(all) => self.artists = artist_groups(all),
            None if !self.artists.is_empty() => self.artists.resize(n, 0),
            None => {}
        }
        if self.traits.len() == old_n {
            self.traits.resize(n, Traits::default());
        }
        let cursor = self.cursor.min(self.order.len());
        let mut played = vec![false; n];
        for &i in &self.order[..cursor] {
            played[i] = true;
        }
        if let Some(l) = self.last_ctx.filter(|l| *l < n) {
            played[l] = true;
        }
        let heard: Vec<usize> = (0..old_n).filter(|i| played[*i]).collect();
        let new_idx: Vec<usize> = (old_n..n).collect();
        let mut added = self.pool(&new_idx, &heard, self.shuffle);
        if self.shuffle {
            let prev = self.order.last().map_or(0, |&i| self.artist_of(i));
            balanced_shuffle(
                &mut added,
                &self.artists,
                prev,
                lap_seed(self.seed, self.lap) ^ (old_n as u64),
            );
        }
        self.order.extend(added);
        self.bump();
        n - old_n
    }

    // ── Karıştırma ve tekrar ───────────────────────────────────────────

    /// Karıştırmayı açar / kapatır. Çalan şarkı değişmez. Açılınca bağlamın
    /// kalanı (çalınanlar hariç) karışır; kapanınca bağlamın kendi sırasına,
    /// çalan şarkıdan sonrasına dönülür. Geçmiş korunur.
    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        if self.ctx.is_empty() {
            return;
        }
        let first = self.last_ctx;
        self.rebuild_order(first);
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    /// `lap` turunun çalma sırası (karıştırma kapalıyken kimlik). Karıştırma
    /// açıkken yeni turun ilk şarkısı bir önce çalanla aynı olmaz.
    fn lap_order(&self, lap: u32) -> Vec<usize> {
        let n = self.ctx.len();
        let all: Vec<usize> = (0..n).collect();
        let mut order = self.pool(&all, &[], self.shuffle);
        if self.shuffle {
            let prev = self.last_ctx.map_or(0, |i| self.artist_of(i));
            balanced_shuffle(&mut order, &self.artists, prev, lap_seed(self.seed, lap));
            if order.len() > 1 && self.last_ctx == Some(order[0]) {
                order.swap(0, 1);
            }
        }
        order
    }

    fn wrap(&mut self) {
        let lap = self.lap.wrapping_add(1);
        self.order = self.lap_order(lap);
        self.lap = lap;
        self.cursor = 0;
        self.bump();
    }

    // ── İlerleme ───────────────────────────────────────────────────────

    fn push_history(&mut self, p: Played) {
        if self.history.len() >= HISTORY_LIMIT {
            self.history.pop_front();
        }
        self.history.push_back(p);
    }

    /// Sıradaki şarkıyı çıkarır; yoksa `None` ve durum değişmez. Tekrar
    /// "hepsi"yse bağlam bitince başa sarar.
    fn take_next(&mut self) -> Option<Played> {
        if let Some(u) = self.user.pop_front() {
            return Some(Played {
                uri: u.uri,
                origin: Origin::User,
                track: u.track,
            });
        }
        if self.cursor >= self.order.len() {
            if self.repeat == RepeatMode::All && !self.ctx.is_empty() {
                self.wrap();
            } else {
                return None;
            }
        }
        let idx = *self.order.get(self.cursor)?;
        self.cursor += 1;
        Some(Played {
            uri: self.ctx[idx].clone(),
            origin: Origin::Context(idx),
            track: None,
        })
    }

    fn become_current(&mut self, next: Played) -> String {
        if let Some(cur) = self.current.take() {
            self.push_history(cur);
        }
        if let Origin::Context(i) = next.origin {
            self.last_ctx = Some(i);
        }
        let uri = next.uri.clone();
        self.current = Some(next);
        uri
    }

    /// Kullanıcının "Sonraki"si (ve çalınamayan şarkıyı atlama): tekrar
    /// "tek" olsa da ilerler. Yeni şarkının adresi; yoksa `None`.
    pub fn next(&mut self) -> Option<String> {
        let next = self.take_next()?;
        Some(self.become_current(next))
    }

    /// Şarkı kendiliğinden bitti: tekrar "tek"se aynı şarkı, değilse sıradaki.
    pub fn next_auto(&mut self) -> Option<String> {
        if self.repeat == RepeatMode::One
            && let Some(cur) = &self.current
        {
            return Some(cur.uri.clone());
        }
        self.next()
    }

    /// Önceki şarkı: geçmişten döner; geçmiş boşsa (ör. albümün ortasından
    /// başlandı) bağlamın bir önceki şarkısına gider. Yoksa `None`.
    pub fn previous(&mut self) -> Option<String> {
        if let Some(prev) = self.history.pop_back() {
            if let Some(cur) = self.current.take() {
                self.unstep(cur);
            }
            if let Origin::Context(i) = prev.origin {
                self.last_ctx = Some(i);
            }
            let uri = prev.uri.clone();
            self.current = Some(prev);
            return Some(uri);
        }
        let ci = self.current_ctx()?;
        if self.cursor >= 2 && self.order.get(self.cursor - 1) == Some(&ci) {
            let pi = self.order[self.cursor - 2];
            self.cursor -= 1;
            self.last_ctx = Some(pi);
            let uri = self.ctx[pi].clone();
            self.current = Some(Played {
                uri: uri.clone(),
                origin: Origin::Context(pi),
                track: None,
            });
            return Some(uri);
        }
        None
    }

    /// Çalan şarkı geri alınırken sıranın başına döner (Sonraki onu çalar).
    fn unstep(&mut self, cur: Played) {
        match cur.origin {
            Origin::User => self.user.push_front(UserItem {
                uri: cur.uri,
                track: cur.track,
            }),
            Origin::Context(ci) => {
                if self.cursor > 0 && self.order.get(self.cursor - 1) == Some(&ci) {
                    self.cursor -= 1;
                } else {
                    self.order.insert(self.cursor.min(self.order.len()), ci);
                    self.bump();
                }
            }
        }
    }

    /// Sonraki'ye basılınca çalacak şarkı var mı?
    pub fn has_next(&self) -> bool {
        !self.user.is_empty()
            || self.cursor < self.order.len()
            || (self.repeat == RepeatMode::All && self.any_playable())
    }

    pub fn has_previous(&self) -> bool {
        if !self.history.is_empty() {
            return true;
        }
        match self.current_ctx() {
            Some(ci) => self.cursor >= 2 && self.order.get(self.cursor - 1) == Some(&ci),
            None => false,
        }
    }

    /// Sıradaki şarkı kendiliğinden geçişte çalacak olan: boşluksuz geçiş
    /// için önceden yüklenir. Tekrar "tek"te yok (çalar aynı şarkıyı kendi
    /// belleğinden yeniden başlatır, bkz. motor).
    pub fn peek_next_auto(&self) -> Option<String> {
        if self.repeat == RepeatMode::One {
            return None;
        }
        if let Some(u) = self.user.front() {
            return Some(u.uri.clone());
        }
        if let Some(i) = self.order.get(self.cursor) {
            return self.ctx.get(*i).cloned();
        }
        if self.repeat == RepeatMode::All && !self.ctx.is_empty() {
            let first = *self.lap_order(self.lap.wrapping_add(1)).first()?;
            return self.ctx.get(first).cloned();
        }
        None
    }

    // ── Kullanıcı sırası ───────────────────────────────────────────────

    /// Sıranın sonuna ekler (bağlamdan önce, kullanıcı sırasının arkasına).
    pub fn enqueue(&mut self, items: Vec<UserItem>) {
        for item in items {
            if self.user.len() >= USER_QUEUE_LIMIT {
                break;
            }
            self.user.push_back(item);
        }
    }

    /// "Sonra çal": verilen sırayla kullanıcı sırasının başına koyar.
    pub fn play_next(&mut self, items: Vec<UserItem>) {
        let room = USER_QUEUE_LIMIT.saturating_sub(self.user.len());
        for item in items
            .into_iter()
            .take(room)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            self.user.push_front(item);
        }
    }

    /// Görünür listenin (önce kullanıcı sırası, sonra bağlamın kalanı)
    /// uzunluğu.
    fn visible_len(&self) -> usize {
        self.user.len() + self.order.len().saturating_sub(self.cursor)
    }

    /// Görünür listeden `i`. satırı kaldırır.
    pub fn remove(&mut self, i: usize) -> bool {
        let ulen = self.user.len();
        if i < ulen {
            self.user.remove(i);
            return true;
        }
        let pos = self.cursor + (i - ulen);
        if pos < self.order.len() {
            self.order.remove(pos);
            self.bump();
            true
        } else {
            false
        }
    }

    /// Görünür listede `from`. satırı `to`. yere taşır (taşındıktan sonraki
    /// konum). Kullanıcı şarkısı kullanıcı sırasında kalır (sonuna kırpılır);
    /// bağlam şarkısı bağlam içinde yer değiştirir ya da kullanıcı sırasına
    /// girerse kullanıcı şarkısına dönüşür.
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        let total = self.visible_len();
        if from >= total {
            return false;
        }
        let to = to.min(total - 1);
        if from == to {
            return true;
        }
        let ulen = self.user.len();
        if from < ulen {
            let item = self.user.remove(from).expect("indeks denetlendi");
            let pos = to.min(self.user.len());
            self.user.insert(pos, item);
        } else {
            let ci = self.order.remove(self.cursor + (from - ulen));
            let ulen2 = self.user.len();
            if to < ulen2 {
                let uri = self.ctx[ci].clone();
                let track = self.rows.as_ref().and_then(|r| r.get(ci).cloned());
                self.user.insert(to, UserItem { uri, track });
            } else {
                let at = (self.cursor + (to - ulen2)).min(self.order.len());
                self.order.insert(at, ci);
            }
            self.bump();
        }
        true
    }

    /// Kullanıcı sırasını boşaltır (bağlam kalır).
    pub fn clear_user(&mut self) {
        self.user.clear();
    }

    /// Görünür listedeki `i`. şarkıya atla: öncesindeki kullanıcı şarkıları
    /// sıradan düşer; bağlam şarkısına atlanırsa aradaki bağlam şarkıları
    /// atlanır (kullanıcı sırası korunur).
    pub fn jump_to(&mut self, i: usize) -> Option<String> {
        let ulen = self.user.len();
        if i < ulen {
            self.user.drain(..i);
        } else {
            let skip = i - ulen;
            if self.cursor + skip >= self.order.len() {
                return None;
            }
            self.cursor += skip;
            // Atlanan bağlam şarkıları çalınmış sayılır (geçmişe yazılmaz);
            // kullanıcı sırası yerinde kalır, bu şarkıdan sonra çalar.
            let idx = self.order[self.cursor];
            self.cursor += 1;
            let played = Played {
                uri: self.ctx[idx].clone(),
                origin: Origin::Context(idx),
                track: None,
            };
            return Some(self.become_current(played));
        }
        self.next()
    }

    // ── Görünüm ────────────────────────────────────────────────────────

    fn ctx_entry(
        &self,
        idx: usize,
        resolve: &dyn Fn(&str) -> Option<TrackRow>,
    ) -> Option<QueueEntry> {
        let uri = self.ctx.get(idx)?;
        let track = self
            .rows
            .as_ref()
            .and_then(|r| r.get(idx).cloned())
            .or_else(|| resolve(uri));
        Some(QueueEntry {
            uri: uri.clone(),
            track,
            source: QueueSource::Context,
        })
    }

    /// Arayüze giden görünüm. `resolve`: bilgisi sırada olmayan şarkılar için
    /// motorun önbelleği.
    pub fn view(&self, resolve: &dyn Fn(&str) -> Option<TrackRow>) -> QueueView {
        let current = self.current.as_ref().map(|p| {
            let (track, source) = match p.origin {
                Origin::Context(i) => (
                    self.rows.as_ref().and_then(|r| r.get(i).cloned()),
                    QueueSource::Context,
                ),
                Origin::User => (p.track.clone(), QueueSource::User),
            };
            QueueEntry {
                uri: p.uri.clone(),
                track: track.or_else(|| resolve(&p.uri)),
                source,
            }
        });
        let mut upcoming: Vec<QueueEntry> = self
            .user
            .iter()
            .map(|u| QueueEntry {
                uri: u.uri.clone(),
                track: u.track.clone().or_else(|| resolve(&u.uri)),
                source: QueueSource::User,
            })
            .collect();
        let user_len = upcoming.len();
        let remaining = self.order.get(self.cursor..).unwrap_or(&[]);
        upcoming.extend(
            remaining
                .iter()
                .take(VIEW_UPCOMING)
                .filter_map(|i| self.ctx_entry(*i, resolve)),
        );
        // Tekrar "hepsi": bağlam bitince sıradaki tur görünür.
        let shown = upcoming.len() - user_len;
        if self.repeat == RepeatMode::All && shown < VIEW_UPCOMING && !self.ctx.is_empty() {
            let next_lap = self.lap_order(self.lap.wrapping_add(1));
            upcoming.extend(
                next_lap
                    .iter()
                    .take(VIEW_UPCOMING - shown)
                    .filter_map(|i| self.ctx_entry(*i, resolve)),
            );
        }
        QueueView {
            current,
            upcoming,
            user_len,
            context_remaining: remaining.len(),
            context: self.context.clone(),
            shuffle: self.shuffle,
            repeat: self.repeat,
            radio: self.radio.clone(),
        }
    }

    /// Görünümdeki bilgisi bulunamamış şarkıların adresleri.
    pub fn missing_meta(view: &QueueView) -> Vec<String> {
        view.current
            .iter()
            .chain(view.upcoming.iter())
            .filter(|e| e.track.is_none())
            .map(|e| e.uri.clone())
            .collect()
    }

    /// Bağlamın (özgün sırasıyla) şarkı adresleri.
    #[cfg(test)]
    pub fn context_uris(&self) -> &[String] {
        &self.ctx
    }

    /// Bağlamın kalanının (çalma sırasıyla) adresleri; testler ve kayıt için.
    #[cfg(test)]
    fn upcoming_context(&self) -> Vec<String> {
        self.order[self.cursor.min(self.order.len())..]
            .iter()
            .map(|i| self.ctx[*i].clone())
            .collect()
    }

    /// Kaydedilmiş durumun tutarlı olduğunu denetler (bozuk kayıt çökertmesin).
    pub(super) fn is_consistent(&self) -> bool {
        let n = self.ctx.len();
        self.cursor <= self.order.len()
            && (self.artists.is_empty() || self.artists.len() == n)
            && (self.traits.is_empty() || self.traits.len() == n)
            && self.order.iter().all(|i| *i < n)
            && self.last_ctx.is_none_or(|i| i < n)
            && self
                .current
                .iter()
                .chain(self.history.iter())
                .all(|p| match p.origin {
                    Origin::Context(i) => i < n,
                    Origin::User => true,
                })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uris(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("t{i}")).collect()
    }

    fn item(uri: &str) -> UserItem {
        UserItem {
            uri: uri.into(),
            track: None,
        }
    }

    /// `n` şarkılı bağlam, `index`'ten başlamış sıra.
    fn started(n: usize, index: usize) -> Queue {
        let mut q = Queue::new();
        q.restart(None, uris(n), None, index, 1, true).unwrap();
        q
    }

    fn cur(q: &Queue) -> &str {
        q.current_uri().unwrap()
    }

    /// Sıranın sonuna kadar `next()` ile gezip adresleri toplar.
    fn drain(q: &mut Queue) -> Vec<String> {
        let mut out = vec![];
        while let Some(u) = q.next() {
            out.push(u);
            assert!(out.len() < 1000, "sonsuz döngü");
        }
        out
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    // ── temel ilerleme ──

    #[test]
    fn state_edges() {
        let q = started(3, 0);
        assert!(q.has_next() && !q.has_previous());
        let q = started(3, 2);
        assert!(!q.has_next() && q.has_previous());
        let q = started(1, 0);
        assert!(!q.has_next() && !q.has_previous());
        assert!(Queue::new().is_idle() && !Queue::new().has_next());
    }

    #[test]
    fn next_walks_context_in_order_then_stops() {
        let mut q = started(4, 1);
        assert_eq!(drain(&mut q), ["t2", "t3"]);
        assert_eq!(cur(&q), "t3");
        assert!(q.next().is_none());
        assert_eq!(cur(&q), "t3", "başarısız Sonraki durumu bozmaz");
    }

    #[test]
    fn previous_uses_history_then_context_order() {
        let mut q = started(6, 3);
        // Albümün ortasından başlandı: geçmiş boş ama bağlamda önceki var.
        assert!(q.has_previous());
        assert_eq!(q.previous().as_deref(), Some("t2"));
        assert_eq!(q.previous().as_deref(), Some("t1"));
        // Geri gidilen şarkılar ileri giderken yine sırayla gelir.
        assert_eq!(q.next().as_deref(), Some("t2"));
        assert_eq!(q.next().as_deref(), Some("t3"));
        assert_eq!(q.next().as_deref(), Some("t4"));
        assert_eq!(q.previous().as_deref(), Some("t3"));
        assert_eq!(q.previous().as_deref(), Some("t2"));
        assert_eq!(q.previous().as_deref(), Some("t1"));
        assert_eq!(q.previous().as_deref(), Some("t0"));
        assert!(!q.has_previous());
        assert!(q.previous().is_none());
        assert_eq!(cur(&q), "t0");
    }

    #[test]
    fn previous_then_next_roundtrip_is_identity() {
        let mut q = started(5, 0);
        q.next();
        q.next(); // t2
        let before = q.upcoming_context();
        q.previous();
        assert_eq!(cur(&q), "t1");
        q.next();
        assert_eq!(cur(&q), "t2");
        assert_eq!(q.upcoming_context(), before);
    }

    // ── kullanıcı sırası ──

    #[test]
    fn user_queue_plays_before_context_then_resumes_context() {
        let mut q = started(4, 0);
        q.enqueue(vec![item("u1"), item("u2")]);
        assert_eq!(drain(&mut q), ["u1", "u2", "t1", "t2", "t3"]);
    }

    #[test]
    fn play_next_goes_to_front_in_given_order() {
        let mut q = started(3, 0);
        q.enqueue(vec![item("u1")]);
        q.play_next(vec![item("p1"), item("p2")]);
        assert_eq!(drain(&mut q)[..4], ["p1", "p2", "u1", "t1"]);
    }

    #[test]
    fn previous_returns_queued_track_to_front() {
        let mut q = started(3, 0);
        q.enqueue(vec![item("u1")]);
        assert_eq!(q.next().as_deref(), Some("u1"));
        // u1'den Önceki: t0'a döner, u1 yine sıradaki olur.
        assert_eq!(q.previous().as_deref(), Some("t0"));
        assert_eq!(q.next().as_deref(), Some("u1"));
        assert_eq!(q.next().as_deref(), Some("t1"));
        // t1'den Önceki: u1; sonra t1 tekrar.
        assert_eq!(q.previous().as_deref(), Some("u1"));
        assert_eq!(q.next().as_deref(), Some("t1"));
        assert_eq!(q.next().as_deref(), Some("t2"));
    }

    #[test]
    fn enqueue_without_context_starts_with_next() {
        let mut q = Queue::new();
        q.enqueue(vec![item("u1"), item("u2")]);
        assert!(q.has_next() && !q.has_previous());
        assert_eq!(q.next().as_deref(), Some("u1"));
        assert_eq!(q.next().as_deref(), Some("u2"));
        assert!(q.next().is_none());
        assert_eq!(q.previous().as_deref(), Some("u1"));
    }

    #[test]
    fn user_queue_is_capped() {
        let mut q = Queue::new();
        q.enqueue(
            (0..USER_QUEUE_LIMIT + 20)
                .map(|i| item(&format!("u{i}")))
                .collect(),
        );
        assert_eq!(q.user_len(), USER_QUEUE_LIMIT);
        q.play_next(vec![item("x")]);
        assert_eq!(
            q.user_len(),
            USER_QUEUE_LIMIT,
            "dolu sıraya sonra çal eklenmez"
        );
    }

    #[test]
    fn remove_and_clear() {
        let mut q = started(5, 0);
        q.enqueue(vec![item("u1"), item("u2"), item("u3")]);
        assert!(q.remove(1)); // u2
        assert!(q.remove(3)); // görünür: u1 u3 t1 t2 t3 t4 → t2
        assert!(!q.remove(99));
        assert_eq!(drain(&mut q), ["u1", "u3", "t1", "t3", "t4"]);

        let mut q = started(3, 0);
        q.enqueue(vec![item("u1"), item("u2")]);
        q.clear_user();
        assert_eq!(q.user_len(), 0);
        assert_eq!(drain(&mut q), ["t1", "t2"], "bağlam yerinde kalır");
    }

    #[test]
    fn move_within_user_queue() {
        let mut q = started(2, 0);
        q.enqueue(vec![item("a"), item("b"), item("c")]);
        assert!(q.move_item(0, 2)); // a → sona: b c a
        assert_eq!(drain(&mut q), ["b", "c", "a", "t1"]);
    }

    #[test]
    fn move_user_item_into_context_region_stays_at_end_of_user_queue() {
        let mut q = started(4, 0);
        q.enqueue(vec![item("a"), item("b")]);
        assert!(q.move_item(0, 5));
        assert_eq!(drain(&mut q), ["b", "a", "t1", "t2", "t3"]);
    }

    #[test]
    fn move_within_context_and_promote_to_user() {
        let mut q = started(5, 0); // kalan: t1 t2 t3 t4
        assert!(q.move_item(3, 0)); // t4 öne
        assert_eq!(q.upcoming_context(), ["t4", "t1", "t2", "t3"]);

        // Bağlam şarkısını kullanıcı sırasına sürükle: kullanıcı şarkısı olur.
        let mut q = started(4, 0);
        q.enqueue(vec![item("a")]);
        assert!(q.move_item(3, 0)); // t3 → en öne (kullanıcı bölgesi)
        assert_eq!(drain(&mut q), ["t3", "a", "t1", "t2"]);
    }

    #[test]
    fn move_out_of_range() {
        let mut q = started(3, 0);
        assert!(!q.move_item(9, 0));
        assert!(q.move_item(0, 99), "hedef son satıra kırpılır");
        assert_eq!(q.upcoming_context(), ["t2", "t1"]);
    }

    #[test]
    fn jump_to_user_and_context_items() {
        let mut q = started(5, 0);
        q.enqueue(vec![item("a"), item("b"), item("c")]);
        assert_eq!(q.jump_to(1).as_deref(), Some("b")); // a düşer
        assert_eq!(drain(&mut q)[..3], ["c", "t1", "t2"]);

        let mut q = started(5, 0);
        q.enqueue(vec![item("a")]);
        // görünür: a t1 t2 t3 t4; t3'e atla (i = 3): t1, t2 atlanır
        assert_eq!(q.jump_to(3).as_deref(), Some("t3"));
        assert_eq!(drain(&mut q), ["a", "t4"], "kullanıcı sırası korunur");
        assert!(started(2, 0).jump_to(9).is_none());
    }

    // ── karıştırma ──

    #[test]
    fn shuffle_is_a_permutation_with_current_first() {
        let mut q = started(30, 7);
        q.set_shuffle(true);
        assert_eq!(cur(&q), "t7", "çalan şarkı değişmez");
        let rest = drain(&mut q);
        assert_eq!(rest.len(), 29);
        let mut all = rest.clone();
        all.push("t7".into());
        assert_eq!(sorted(all), sorted(uris(30)), "her şarkı tam bir kez");
        assert_ne!(
            rest,
            uris(30)
                .into_iter()
                .filter(|u| u != "t7")
                .collect::<Vec<_>>(),
            "gerçekten karışmış"
        );
    }

    #[test]
    fn shuffle_is_deterministic_for_a_seed() {
        let order = |seed| {
            let mut q = Queue::new();
            q.set_shuffle(true);
            q.restart(None, uris(40), None, 0, seed, true);
            q.upcoming_context()
        };
        assert_eq!(order(5), order(5));
        assert_ne!(order(5), order(6));
    }

    #[test]
    fn shuffle_off_returns_to_context_order_from_current() {
        let mut q = started(10, 0);
        q.set_shuffle(true);
        q.next();
        q.next();
        let here: usize = cur(&q)[1..].parse().unwrap();
        q.set_shuffle(false);
        assert_eq!(cur(&q), format!("t{here}"));
        let expected: Vec<String> = (here + 1..10).map(|i| format!("t{i}")).collect();
        assert_eq!(drain(&mut q), expected);
    }

    #[test]
    fn shuffle_on_off_on_gives_the_same_order() {
        let mut q = started(25, 3);
        q.set_shuffle(true);
        let a = q.upcoming_context();
        q.set_shuffle(false);
        q.set_shuffle(true);
        assert_eq!(q.upcoming_context(), a, "kararlı permütasyon");
    }

    #[test]
    fn shuffle_excludes_already_played_and_keeps_history() {
        let mut q = started(8, 0);
        q.next();
        q.next();
        q.next(); // t3 çalıyor; t0 t1 t2 geçmişte
        q.set_shuffle(true);
        let rest = q.upcoming_context();
        assert_eq!(
            sorted(rest.clone()),
            ["t4", "t5", "t6", "t7"],
            "çalınanlar karışmaya girmez"
        );
        assert!(q.has_previous());
        assert_eq!(q.previous().as_deref(), Some("t2"));
        assert_eq!(q.previous().as_deref(), Some("t1"));
        // t1'den ileri: t2, t3, sonra karışık sıranın ilki
        assert_eq!(q.next().as_deref(), Some("t2"));
        assert_eq!(q.next().as_deref(), Some("t3"));
        assert_eq!(q.next().as_deref(), Some(rest[0].as_str()));
    }

    #[test]
    fn shuffle_with_user_item_playing() {
        let mut q = started(6, 2);
        q.enqueue(vec![item("u1")]);
        q.next(); // u1 çalıyor, bağlamda son çalan t2
        q.set_shuffle(true);
        assert_eq!(cur(&q), "u1");
        let rest = drain(&mut q);
        assert_eq!(rest.len(), 5, "t2 tekrar gelmez");
        assert!(!rest.contains(&"t2".to_string()));
        q.set_shuffle(false);
        assert_eq!(cur(&q), rest.last().unwrap().as_str());
    }

    #[test]
    fn shuffle_applies_to_new_contexts_with_clicked_track_first() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.restart(None, uris(12), None, 5, 9, true);
        assert_eq!(cur(&q), "t5");
        assert_eq!(q.upcoming_context().len(), 11);
        assert!(!q.upcoming_context().contains(&"t5".to_string()));
        assert!(q.shuffle());
    }

    #[test]
    fn fisher_yates_is_unbiased_enough() {
        // 3 öğenin 6 permütasyonu da çıkmalı, kabaca eşit sıklıkta.
        let mut counts = std::collections::HashMap::new();
        for seed in 0..6000u64 {
            let mut v = [0, 1, 2];
            fisher_yates(&mut v, seed.wrapping_mul(0x9E37_79B9));
            *counts.entry(v).or_insert(0) += 1;
        }
        assert_eq!(counts.len(), 6);
        for c in counts.values() {
            assert!((700..1300).contains(c), "dağılım dengesiz: {counts:?}");
        }
    }

    // ── dengeli karıştırma ──

    /// `t{i}` adresli, `artists[i]` sanatçılı satırlar ("" sanatçısız).
    fn rows_by(artists: &[String]) -> Arc<Vec<TrackRow>> {
        Arc::new(
            artists
                .iter()
                .enumerate()
                .map(|(i, a)| TrackRow {
                    uri: format!("t{i}"),
                    name: format!("Şarkı {i}"),
                    artists: a.clone(),
                    album: String::new(),
                    duration_ms: 1000,
                    image: String::new(),
                    plays: None,
                })
                .collect(),
        )
    }

    /// Sanatçılı bağlam, karıştırma açık, `index`'ten başlamış sıra.
    fn shuffled(artists: &[String], index: usize, seed: u64) -> Queue {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.restart(
            None,
            uris(artists.len()),
            Some(rows_by(artists)),
            index,
            seed,
            true,
        )
        .unwrap();
        q
    }

    /// Çalan + bağlamın kalanı (çalma sırasıyla), sanatçı adlarıyla.
    fn play_artists(q: &Queue, artists: &[String]) -> Vec<String> {
        let idx = |u: &str| u[1..].parse::<usize>().unwrap();
        std::iter::once(cur(q).to_string())
            .chain(q.upcoming_context())
            .map(|u| artists[idx(&u)].clone())
            .collect()
    }

    /// Art arda aynı (bilinen) sanatçı sayısı.
    fn adjacent_repeats(seq: &[String]) -> usize {
        seq.windows(2)
            .filter(|w| !w[0].is_empty() && w[0] == w[1])
            .count()
    }

    /// İlk şarkısı sabit (`first` sanatçısı) dizi art arda gelmeden kurulabilir mi?
    /// (Doğrudan sayımla; `Counts`'tan bağımsız.)
    fn possible(artists: &[String], first: usize) -> bool {
        let mut counts: std::collections::HashMap<&str, usize> = Default::default();
        for (i, a) in artists.iter().enumerate() {
            if i != first && !a.is_empty() {
                *counts.entry(a.as_str()).or_default() += 1;
            }
        }
        let rest = artists.len() - 1;
        counts.iter().all(|(a, c)| {
            if *a == artists[first].as_str() {
                *c <= rest / 2
            } else {
                *c <= rest.div_ceil(2)
            }
        })
    }

    /// Tohumdan rastgele liste: 1–8 sanatçı (çarpık dağılımlı), bir kısmı
    /// sanatçısız.
    fn random_playlist(seed: u64) -> Vec<String> {
        let mut s = seed;
        let n = 2 + bounded(&mut s, 60);
        let k = 1 + bounded(&mut s, 8);
        (0..n)
            .map(|_| {
                if bounded(&mut s, 10) == 0 {
                    return String::new();
                }
                // Çarpık: küçük numaralı sanatçılar daha sık.
                let a = bounded(&mut s, k).min(bounded(&mut s, k));
                format!("Sanatçı {a}")
            })
            .collect()
    }

    #[test]
    fn balanced_shuffle_never_repeats_an_artist_when_avoidable() {
        let mut checked = 0;
        for seed in 0..2000u64 {
            let artists = random_playlist(seed);
            let first = (seed as usize * 7) % artists.len();
            if !possible(&artists, first) {
                continue;
            }
            checked += 1;
            let q = shuffled(&artists, first, seed ^ 0x5EED);
            let seq = play_artists(&q, &artists);
            assert_eq!(adjacent_repeats(&seq), 0, "tohum {seed}: {seq:?}");
            let mut all: Vec<String> = q.upcoming_context();
            all.push(cur(&q).to_string());
            assert_eq!(
                sorted(all),
                sorted(uris(artists.len())),
                "tohum {seed}: her şarkı tam bir kez"
            );
        }
        assert!(checked > 500, "yeterince olası durum sınanmadı: {checked}");

        // Sınırdaki durumlar: tek bir dizilim mümkün (A B A B … A).
        let tight: Vec<String> = (0..9)
            .map(|i| if i % 2 == 0 { "A" } else { "B" }.to_string())
            .collect();
        for seed in 0..200 {
            let q = shuffled(&tight, 0, seed);
            assert_eq!(
                adjacent_repeats(&play_artists(&q, &tight)),
                0,
                "dar durum, tohum {seed}"
            );
        }
        // Yarı yarıya iki sanatçı, B'den başlanır.
        let half: Vec<String> = (0..20)
            .map(|i| if i < 10 { "A" } else { "B" }.to_string())
            .collect();
        for seed in 0..200 {
            let q = shuffled(&half, 15, seed);
            assert_eq!(
                adjacent_repeats(&play_artists(&q, &half)),
                0,
                "yarı yarıya, tohum {seed}"
            );
        }
    }

    #[test]
    fn balanced_shuffle_spreads_unavoidable_repeats() {
        // 8 A + 2 B: art arda A kaçınılmaz. B'den başlanınca en az 6, A'dan
        // başlanınca en az 5 (A'lar en çok 2 / 3 öbeğe bölünebilir); her
        // tohumda tam bu kadar, B'ler hiç yan yana değil, A öbekleri eşit.
        let artists: Vec<String> = (0..10)
            .map(|i| if i < 8 { "A" } else { "B" }.to_string())
            .collect();
        for (start, least) in [(8, 6), (0, 5)] {
            for seed in 0..300 {
                let q = shuffled(&artists, start, seed);
                let seq = play_artists(&q, &artists);
                assert_eq!(
                    adjacent_repeats(&seq),
                    least,
                    "başlangıç {start}, tohum {seed}: {seq:?}"
                );
                assert!(
                    !seq.windows(2).any(|w| w[0] == "B" && w[1] == "B"),
                    "tohum {seed}: {seq:?}"
                );
                let runs: Vec<usize> = seq
                    .chunk_by(|a, b| a == b)
                    .filter(|r| r[0] == "A")
                    .map(<[String]>::len)
                    .collect();
                let (lo, hi) = (runs.iter().min().unwrap(), runs.iter().max().unwrap());
                assert!(hi - lo <= 1, "A öbekleri eşit değil: {seq:?}");
            }
        }
    }

    #[test]
    fn balanced_shuffle_spreads_each_artist_evenly() {
        // 120 şarkı: A ve B 12'şer, kalan 96'sı ayrı sanatçı. Beklenen A
        // aralığı 10. Düz karıştırmada en dar aralık çoğunlukla 1, en geniş
        // 25'i aşar; dengelide her tohumda dar ve geniş aralık sınırlı.
        let artists: Vec<String> = (0..120)
            .map(|i| match i {
                0..=11 => "A".to_string(),
                12..=23 => "B".to_string(),
                _ => format!("Tek {i}"),
            })
            .collect();
        let gaps = |seq: &[String], who: &str| -> (usize, usize) {
            let at: Vec<usize> = seq
                .iter()
                .enumerate()
                .filter(|(_, a)| *a == who)
                .map(|(i, _)| i)
                .collect();
            let g: Vec<usize> = at.windows(2).map(|w| w[1] - w[0]).collect();
            (*g.iter().min().unwrap(), *g.iter().max().unwrap())
        };
        let (mut min_gap, mut max_gap) = (usize::MAX, 0);
        for seed in 0..300 {
            let q = shuffled(&artists, 100, seed);
            let seq = play_artists(&q, &artists);
            for who in ["A", "B"] {
                let (lo, hi) = gaps(&seq, who);
                min_gap = min_gap.min(lo);
                max_gap = max_gap.max(hi);
            }
        }
        eprintln!("dengeli: en dar aralık {min_gap}, en geniş {max_gap} (ideal 10)");
        assert!(min_gap >= 4, "en dar aralık {min_gap}");
        assert!(max_gap <= 18, "en geniş aralık {max_gap}");

        // Karşılaştırma: aynı listede düz Fisher-Yates.
        let mut plain_min = usize::MAX;
        for seed in 0..300 {
            let mut order: Vec<usize> = (0..120).collect();
            fisher_yates(&mut order, seed);
            let seq: Vec<String> = order.iter().map(|i| artists[*i].clone()).collect();
            plain_min = plain_min.min(gaps(&seq, "A").0);
        }
        eprintln!("düz Fisher-Yates: en dar aralık {plain_min}");
        assert_eq!(plain_min, 1, "düz karıştırma A'yı art arda getirir");
    }

    #[test]
    fn balanced_shuffle_across_repeat_laps_and_after_toggle() {
        // 4 sanatçı × 5 şarkı: her turun içinde ve tur geçişinde art arda yok.
        let artists: Vec<String> = (0..20).map(|i| format!("S{}", i % 4)).collect();
        for seed in 0..50 {
            let mut q = shuffled(&artists, 3, seed);
            q.set_repeat(RepeatMode::All);
            let idx = |u: &str| u[1..].parse::<usize>().unwrap();
            let mut seq = vec![artists[idx(cur(&q))].clone()];
            for _ in 0..79 {
                seq.push(artists[idx(&q.next_auto().unwrap())].clone());
            }
            assert_eq!(adjacent_repeats(&seq), 0, "tohum {seed}");
        }
        // Karıştırmayı ortada kapatıp açınca da (çalan sabit) art arda yok.
        let mut q = shuffled(&artists, 0, 9);
        q.next();
        q.next();
        q.set_shuffle(false);
        q.set_shuffle(true);
        assert_eq!(adjacent_repeats(&play_artists(&q, &artists)), 0);
    }

    #[test]
    fn unknown_artists_fall_back_to_plain_shuffle() {
        // Satırsız bağlam (sanatçı bilinmiyor): eskisiyle birebir aynı sıra.
        let q = {
            let mut q = Queue::new();
            q.set_shuffle(true);
            q.restart(None, uris(40), None, 0, 77, true);
            q
        };
        let mut expected: Vec<usize> = (1..40).collect();
        fisher_yates(&mut expected, lap_seed(77, 0));
        let expected: Vec<String> = expected.iter().map(|i| format!("t{i}")).collect();
        assert_eq!(q.upcoming_context(), expected);
        assert!(q.artists.is_empty());
    }

    #[test]
    fn artist_groups_use_primary_artist_name() {
        let rows = rows_by(&[
            "Sezen Aksu".into(),
            "sezen aksu, Tarkan".into(),
            "Tarkan".into(),
            "".into(),
            " Tarkan , Sezen Aksu".into(),
        ]);
        assert_eq!(artist_groups(&rows), [1, 1, 2, 0, 2]);
    }

    // ── tekrar ──

    #[test]
    fn repeat_one_replays_on_auto_but_next_advances() {
        let mut q = started(3, 0);
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.next_auto().as_deref(), Some("t0"));
        assert_eq!(q.next_auto().as_deref(), Some("t0"));
        assert_eq!(cur(&q), "t0");
        assert!(!q.has_previous(), "yeniden çalma geçmiş yazmaz");
        assert_eq!(
            q.next().as_deref(),
            Some("t1"),
            "Sonraki tekrar tek'te de ilerler"
        );
        assert_eq!(q.next_auto().as_deref(), Some("t1"));
        assert_eq!(q.peek_next_auto(), None, "tekrar tek'te önyükleme yok");
    }

    #[test]
    fn repeat_all_wraps_and_has_next_at_end() {
        let mut q = started(3, 0);
        q.set_repeat(RepeatMode::All);
        assert_eq!(
            drain_n(&mut q, 7),
            ["t1", "t2", "t0", "t1", "t2", "t0", "t1"]
        );
        assert!(q.has_next());
    }

    fn drain_n(q: &mut Queue, n: usize) -> Vec<String> {
        (0..n).map(|_| q.next_auto().unwrap()).collect()
    }

    #[test]
    fn repeat_all_single_track_replays_itself() {
        let mut q = started(1, 0);
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.next_auto().as_deref(), Some("t0"));
        assert_eq!(q.peek_next_auto().as_deref(), Some("t0"));
    }

    #[test]
    fn repeat_off_stops_at_end() {
        let mut q = started(2, 0);
        assert_eq!(q.next_auto().as_deref(), Some("t1"));
        assert!(q.next_auto().is_none());
        assert!(!q.has_next());
        q.set_repeat(RepeatMode::All);
        assert!(q.has_next());
    }

    #[test]
    fn repeat_all_with_shuffle_reshuffles_each_lap_without_back_to_back() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        q.restart(None, uris(10), None, 0, 42, true);
        let mut laps: Vec<Vec<String>> = vec![];
        let mut seq = vec!["t0".to_string()];
        for _ in 0..39 {
            seq.push(q.next_auto().unwrap());
        }
        // Birinci tur t0 + 9, sonra 3 tam tur.
        laps.push(seq[..10].to_vec());
        for lap in 0..3 {
            laps.push(seq[10 + lap * 10..20 + lap * 10].to_vec());
        }
        for lap in &laps {
            assert_eq!(
                sorted(lap.clone()),
                sorted(uris(10)),
                "her tur eksiksiz bir permütasyon"
            );
        }
        assert!(
            laps[1] != laps[2] || laps[2] != laps[3],
            "turlar yeniden karışır"
        );
        for w in seq.windows(2) {
            assert_ne!(w[0], w[1], "aynı şarkı arka arkaya gelmez");
        }
    }

    #[test]
    fn peek_matches_what_next_will_play_in_every_mode() {
        for shuffle in [false, true] {
            for repeat in [RepeatMode::Off, RepeatMode::All] {
                let mut q = Queue::new();
                q.set_shuffle(shuffle);
                q.set_repeat(repeat);
                q.restart(None, uris(7), None, 2, 77, true);
                q.enqueue(vec![item("u1")]);
                for step in 0..20 {
                    let peek = q.peek_next_auto();
                    let got = q.next_auto();
                    assert_eq!(peek, got, "shuffle={shuffle} repeat={repeat:?} adım {step}");
                    if got.is_none() {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn peek_follows_queue_edits() {
        let mut q = started(4, 0);
        assert_eq!(q.peek_next_auto().as_deref(), Some("t1"));
        q.enqueue(vec![item("u1")]);
        assert_eq!(q.peek_next_auto().as_deref(), Some("u1"));
        q.play_next(vec![item("p")]);
        assert_eq!(q.peek_next_auto().as_deref(), Some("p"));
        q.remove(0);
        q.clear_user();
        assert_eq!(q.peek_next_auto().as_deref(), Some("t1"));
        q.set_shuffle(true);
        assert_eq!(q.peek_next_auto(), q.upcoming_context().first().cloned());
    }

    // ── yeni bağlam ──

    #[test]
    fn new_context_keeps_user_queue_modes_and_history() {
        let mut q = started(4, 0);
        q.set_repeat(RepeatMode::All);
        q.enqueue(vec![item("u1")]);
        q.next(); // u1
        q.restart(
            None,
            vec!["x0".into(), "x1".into(), "x2".into()],
            None,
            1,
            3,
            true,
        );
        assert_eq!(cur(&q), "x1");
        assert_eq!(q.repeat(), RepeatMode::All);
        // Önceki: u1'e döner, oradan t0'a, sonra Sonraki ile x1 yeniden.
        assert_eq!(q.previous().as_deref(), Some("u1"));
        assert_eq!(q.previous().as_deref(), Some("t0"));
        assert_eq!(q.next().as_deref(), Some("u1"));
        assert_eq!(q.next().as_deref(), Some("x1"));
        assert_eq!(q.next().as_deref(), Some("x2"));

        let mut q = started(3, 0);
        q.enqueue(vec![item("u1")]);
        q.restart(None, uris(2), None, 0, 1, false);
        assert_eq!(q.user_len(), 0, "keep_user=false sırayı boşaltır");
    }

    #[test]
    fn restart_rejects_bad_index() {
        let mut q = Queue::new();
        assert!(q.restart(None, uris(2), None, 2, 1, true).is_none());
        assert!(q.is_idle());
    }

    #[test]
    fn history_is_capped() {
        let mut q = started(HISTORY_LIMIT + 50, 0);
        for _ in 0..HISTORY_LIMIT + 40 {
            q.next();
        }
        assert_eq!(q.history.len(), HISTORY_LIMIT);
        let mut back = 0;
        while q.previous().is_some() {
            back += 1;
            assert!(back < 1000);
        }
        // Geçmiş (100) bitince bağlamın kendi sırasında geriye gidilir.
        assert_eq!(back, HISTORY_LIMIT + 40);
        assert_eq!(cur(&q), "t0");
    }

    // ── görünüm ──

    #[test]
    fn view_lists_user_then_context_with_sources() {
        let mut q = started(5, 1);
        q.enqueue(vec![item("u1")]);
        let v = q.view(&|_| None);
        assert_eq!(v.current.as_ref().unwrap().uri, "t1");
        assert_eq!(v.user_len, 1);
        let uris: Vec<_> = v.upcoming.iter().map(|e| e.uri.as_str()).collect();
        assert_eq!(uris, ["u1", "t2", "t3", "t4"]);
        assert_eq!(v.upcoming[0].source, QueueSource::User);
        assert_eq!(v.upcoming[1].source, QueueSource::Context);
        assert_eq!(v.context_remaining, 3);
        assert_eq!(Queue::missing_meta(&v).len(), 5);
    }

    #[test]
    fn view_resolves_meta_from_rows_and_resolver() {
        let row = |u: &str| TrackRow {
            uri: u.into(),
            name: format!("{u} adı"),
            artists: "S".into(),
            album: "A".into(),
            duration_ms: 1000,
            image: String::new(),
            plays: None,
        };
        let rows = Arc::new((0..3).map(|i| row(&format!("t{i}"))).collect::<Vec<_>>());
        let mut q = Queue::new();
        q.restart(Some(CollectionId::Liked), uris(3), Some(rows), 0, 1, true);
        q.enqueue(vec![item("u1")]);
        let v = q.view(&|u| (u == "u1").then(|| row("u1")));
        assert!(Queue::missing_meta(&v).is_empty());
        assert_eq!(v.current.unwrap().track.unwrap().name, "t0 adı");
        assert_eq!(v.context, Some(CollectionId::Liked));
    }

    #[test]
    fn view_shows_next_lap_when_repeating_all() {
        let mut q = started(3, 2);
        q.set_repeat(RepeatMode::All);
        let v = q.view(&|_| None);
        let uris: Vec<_> = v.upcoming.iter().map(|e| e.uri.as_str()).collect();
        assert_eq!(uris, ["t0", "t1", "t2"]);
        assert_eq!(v.context_remaining, 0);
    }

    #[test]
    fn view_window_is_capped() {
        let q = started(500, 0);
        let v = q.view(&|_| None);
        assert_eq!(v.upcoming.len(), VIEW_UPCOMING);
        assert_eq!(v.context_remaining, 499);
    }

    #[test]
    fn index_and_context_for_ui() {
        let mut q = started(4, 2);
        assert_eq!(q.context_index(), Some(2));
        q.enqueue(vec![item("u")]);
        q.next();
        assert_eq!(
            q.context_index(),
            None,
            "kullanıcı şarkısı bağlamdan sayılmaz"
        );
        q.set_shuffle(true);
        q.next();
        let i = q.context_index().unwrap();
        assert_eq!(
            cur(&q),
            format!("t{i}"),
            "karışıkken indeks özgün bağlam sırasındadır"
        );
    }

    #[test]
    fn order_gen_bumps_only_when_order_changes() {
        let mut q = started(5, 0);
        let g = q.order_gen();
        q.next();
        q.enqueue(vec![item("u")]);
        q.next();
        assert_eq!(
            q.order_gen(),
            g,
            "ilerleme ve sıraya ekleme sırayı değiştirmez"
        );
        q.set_shuffle(true);
        assert!(q.order_gen() != g);
        let g = q.order_gen();
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.order_gen(), g);
    }

    // ── karıştırma ve sıra kuralları (bkz. rules.rs) ──

    fn rule_row(i: usize, name: &str, artists: &str, ms: u32) -> TrackRow {
        TrackRow {
            uri: format!("t{i}"),
            name: name.into(),
            artists: artists.into(),
            album: String::new(),
            duration_ms: ms,
            image: String::new(),
            plays: None,
        }
    }

    /// `(ad, sanatçılar)` çiftlerinden bağlam satırları (`t{i}` adresli).
    fn rule_rows(items: &[(&str, &str)]) -> Arc<Vec<TrackRow>> {
        Arc::new(
            items
                .iter()
                .enumerate()
                .map(|(i, (n, a))| rule_row(i, n, a, 200_000))
                .collect(),
        )
    }

    fn started_with(
        rules: Rules,
        shuffle: bool,
        rows: Arc<Vec<TrackRow>>,
        index: usize,
        seed: u64,
    ) -> Queue {
        let mut q = Queue::new();
        q.set_shuffle(shuffle);
        q.set_rules(rules);
        q.restart(None, uris_of(&rows), Some(rows), index, seed, true)
            .unwrap();
        q
    }

    fn uris_of(rows: &[TrackRow]) -> Vec<String> {
        rows.iter().map(|r| r.uri.clone()).collect()
    }

    fn blocked(names: &[&str]) -> Rules {
        Rules::new(false, false, names.iter().copied())
    }

    #[test]
    fn blocked_artist_is_skipped_in_order_and_shuffle() {
        let rows = rule_rows(&[
            ("a", "X"),
            ("b", "Drake"),
            ("c", "Y"),
            ("d", "Future, Drake"),
            ("e", "Z"),
        ]);
        let mut q = started_with(blocked(&["drake"]), false, rows.clone(), 0, 1);
        assert_eq!(
            drain(&mut q),
            ["t2", "t4"],
            "sırayla: kara listedekiler atlanır"
        );
        for seed in 0..50 {
            let mut q = started_with(blocked(&["Drake"]), true, rows.clone(), 0, seed);
            assert_eq!(sorted(drain(&mut q)), ["t2", "t4"], "karışık, tohum {seed}");
        }
    }

    #[test]
    fn explicitly_picked_blocked_track_plays_and_the_queue_continues_past_blocked_ones() {
        let rows = rule_rows(&[
            ("a", "X"),
            ("b", "Drake"),
            ("c", "Y"),
            ("d", "Drake"),
            ("e", "Z"),
        ]);
        // Kullanıcı çift tıkla "b"yi (Drake) seçti: o çalar, ama devamında Drake yok.
        let mut q = started_with(blocked(&["drake"]), false, rows.clone(), 1, 1);
        assert_eq!(cur(&q), "t1");
        assert_eq!(drain(&mut q), ["t2", "t4"]);
        // Karışıkta da çalan şarkı seçilen; kalanında Drake yok.
        let mut q = started_with(blocked(&["drake"]), true, rows, 3, 9);
        assert_eq!(cur(&q), "t3");
        assert_eq!(sorted(drain(&mut q)), ["t0", "t2", "t4"]);
    }

    #[test]
    fn user_queue_is_never_filtered() {
        let rows = rule_rows(&[("a", "X"), ("b", "Drake"), ("c", "Y")]);
        let mut q = started_with(Rules::new(true, true, ["drake"]), false, rows, 0, 1);
        q.enqueue(vec![item("t1"), item("u2")]);
        assert_eq!(
            drain(&mut q),
            ["t1", "u2", "t2"],
            "sıraya kendisi eklediği şarkı çalar"
        );
    }

    #[test]
    fn previous_never_lands_on_a_blocked_track_when_started_mid_album() {
        let rows = rule_rows(&[("a", "X"), ("b", "Drake"), ("c", "Y"), ("d", "Z")]);
        let mut q = started_with(blocked(&["drake"]), false, rows, 2, 1);
        assert_eq!(q.previous().as_deref(), Some("t0"), "t1 (Drake) atlanır");
        assert!(q.previous().is_none());
    }

    #[test]
    fn skip_explicit_uses_known_flags_and_reacts_when_they_arrive() {
        let rows = rule_rows(&[("a", "X"), ("b", "Y"), ("c", "Z"), ("d", "W")]);
        let mut q = started_with(Rules::new(false, true, []), false, rows, 0, 1);
        assert_eq!(
            q.upcoming_context(),
            ["t1", "t2", "t3"],
            "işaret bilinmiyor: düşmez"
        );
        let explicit = Traits {
            explicit: true,
            popularity: 0,
        };
        let found = std::collections::HashMap::from([("t2".to_string(), explicit)]);
        assert!(q.merge_traits(&found));
        assert_eq!(q.upcoming_context(), ["t1", "t3"]);
        assert!(
            !q.merge_traits(&found),
            "aynı bilgi ikinci kez değişiklik sayılmaz"
        );
        assert!(q.is_consistent());
        assert_eq!(drain(&mut q), ["t1", "t3"]);
    }

    #[test]
    fn dedupe_applies_only_to_shuffled_contexts() {
        let rows = rule_rows(&[
            ("Hey Jude", "The Beatles"),
            ("Let It Be", "The Beatles"),
            ("Hey Jude - Remastered 2015", "The Beatles"),
            ("Yesterday", "The Beatles"),
        ]);
        let rules = Rules::new(true, false, []);
        let mut q = started_with(rules.clone(), false, rows.clone(), 1, 1);
        assert_eq!(
            drain(&mut q),
            ["t2", "t3"],
            "sırayla çalarken hiçbir şey elenmez"
        );
        for seed in 0..30 {
            let mut q = started_with(rules.clone(), true, rows.clone(), 1, seed);
            let played = drain(&mut q);
            assert_eq!(played.len(), 2, "iki sürümden biri kalır: {played:?}");
            assert!(played.contains(&"t3".to_string()));
            assert!(played.contains(&"t0".to_string()) ^ played.contains(&"t2".to_string()));
        }
    }

    #[test]
    fn dedupe_keeps_the_most_popular_version_and_drops_versions_of_the_playing_song() {
        let rows = rule_rows(&[
            ("Hey Jude", "The Beatles"),
            ("Hey Jude (Live)", "The Beatles"),
            ("Hey Jude - Remastered 2015", "The Beatles"),
            ("Let It Be", "The Beatles"),
        ]);
        let pop = |p| Traits {
            explicit: false,
            popularity: p,
        };
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.set_rules(Rules::new(true, false, []));
        let traits = vec![pop(40), pop(10), pop(80), pop(50)];
        q.restart_with(
            None,
            uris_of(&rows),
            Some(rows.clone()),
            3,
            5,
            true,
            None,
            traits.clone(),
        )
        .unwrap();
        assert_eq!(
            q.upcoming_context(),
            ["t2"],
            "Let It Be çalıyor; en popüler Hey Jude kalır"
        );
        // Çalan şarkı bir Hey Jude ise öteki sürümlerin hiçbiri çalmaz.
        q.restart_with(
            None,
            uris_of(&rows),
            Some(rows.clone()),
            0,
            5,
            true,
            None,
            traits,
        )
        .unwrap();
        assert_eq!(q.upcoming_context(), ["t3"]);
    }

    #[test]
    fn rules_never_break_the_no_adjacent_artist_guarantee() {
        let mut checked = 0;
        for seed in 0..600u64 {
            let artists = random_playlist(seed);
            let rows = rows_by(&artists);
            let block = format!("Sanatçı {}", bounded(&mut seed.clone(), 8));
            let rules = Rules::new(false, false, [block.as_str()]);
            let first = (seed as usize * 7) % artists.len();
            // Kalan havuzda (çalan şarkı + engellenmeyenler) art arda tekrar
            // kaçınılabiliyor mu?
            let kept: Vec<String> = artists
                .iter()
                .enumerate()
                .filter(|(i, a)| *i == first || **a != block)
                .map(|(_, a)| a.clone())
                .collect();
            let first_in_kept = artists.iter().take(first).filter(|a| **a != block).count();
            if kept.len() < 2 || !possible(&kept, first_in_kept) {
                continue;
            }
            checked += 1;
            let mut q = Queue::new();
            q.set_shuffle(true);
            q.set_rules(rules);
            q.restart(
                None,
                uris(artists.len()),
                Some(rows),
                first,
                seed ^ 0xBEEF,
                true,
            )
            .unwrap();
            let seq = play_artists(&q, &artists);
            assert_eq!(
                adjacent_repeats(&seq),
                0,
                "tohum {seed} (engel {block}): {seq:?}"
            );
            assert!(
                seq.iter().skip(1).all(|a| *a != block),
                "kara listedeki sanatçı çalmamalı: tohum {seed}: {seq:?}"
            );
        }
        assert!(checked > 100, "yeterince olası durum sınanmadı: {checked}");
    }

    #[test]
    fn changing_rules_mid_play_keeps_what_was_played_and_rebuilds_the_rest() {
        let rows = rule_rows(&[
            ("a", "X"),
            ("b", "Drake"),
            ("c", "Y"),
            ("d", "Drake"),
            ("e", "Z"),
            ("f", "Drake"),
        ]);
        let mut q = started_with(Rules::default(), false, rows, 0, 1);
        assert_eq!(q.next().as_deref(), Some("t1"));
        assert!(q.set_rules(blocked(&["drake"])));
        assert_eq!(cur(&q), "t1", "çalan şarkı yerinde");
        assert_eq!(q.upcoming_context(), ["t2", "t4"]);
        assert!(
            !q.set_rules(blocked(&["DRAKE"])),
            "aynı kural değişiklik değil"
        );
        // Kural kalkınca kalan geri gelir, çalınanlar gelmez.
        assert!(q.set_rules(Rules::default()));
        assert_eq!(q.upcoming_context(), ["t2", "t3", "t4", "t5"]);
        assert_eq!(q.previous().as_deref(), Some("t0"), "geçmiş bozulmadı");
    }

    #[test]
    fn changing_rules_in_shuffle_never_replays_or_loses_tracks() {
        let rows = rule_rows(&[
            ("a", "A"),
            ("b", "B"),
            ("c", "C"),
            ("d", "D"),
            ("e", "E"),
            ("f", "F"),
            ("g", "G"),
            ("h", "H"),
        ]);
        for seed in 0..40 {
            let mut q = started_with(Rules::default(), true, rows.clone(), 0, seed);
            let first_three = [cur(&q).to_string(), q.next().unwrap(), q.next().unwrap()];
            q.set_rules(blocked(&["e"]));
            let mut all = first_three.to_vec();
            while let Some(u) = q.next() {
                all.push(u);
            }
            // "e" (t4) çalınmışsa sayılır; çalınmadıysa artık çalmaz.
            let want: Vec<String> = uris(8)
                .into_iter()
                .filter(|u| u != "t4" || first_three.contains(u))
                .collect();
            assert_eq!(
                sorted(all),
                sorted(want),
                "tohum {seed}: hiçbiri tekrarlanmaz ya da kaybolmaz"
            );
        }
    }

    #[test]
    fn repeat_all_laps_keep_obeying_the_rules() {
        let rows = rule_rows(&[("a", "X"), ("b", "Drake"), ("c", "Y"), ("d", "Drake")]);
        for shuffle in [false, true] {
            let mut q = started_with(blocked(&["drake"]), shuffle, rows.clone(), 0, 3);
            q.set_repeat(RepeatMode::All);
            for _ in 0..9 {
                let u = q.next().unwrap();
                assert!(u == "t0" || u == "t2", "tur boyunca kara liste: {u}");
            }
            assert!(q.has_next());
        }
        // Hepsi dışlanmışsa (çalan dışında) "tekrar hepsi" sonsuz boş tur çevirmez.
        let rows = rule_rows(&[("a", "Drake"), ("b", "Drake")]);
        let mut q = started_with(blocked(&["drake"]), false, rows, 0, 1);
        q.set_repeat(RepeatMode::All);
        assert!(!q.has_next());
        assert!(q.next().is_none());
    }

    #[test]
    fn rules_survive_a_new_context_and_an_unknown_context_is_left_alone() {
        let rows = rule_rows(&[("a", "X"), ("b", "Drake"), ("c", "Y")]);
        let mut q = started_with(blocked(&["drake"]), false, rows.clone(), 0, 1);
        q.restart(None, uris(3), Some(rows), 0, 2, true).unwrap();
        assert_eq!(
            q.upcoming_context(),
            ["t2"],
            "yeni bağlamda da kurallar geçerli"
        );
        // Satırsız bağlam (sanatçı bağlamı): adlar bilinmiyor, hiçbir şey elenmez.
        q.restart(None, uris(3), None, 0, 2, true).unwrap();
        assert_eq!(q.upcoming_context(), ["t1", "t2"]);
        assert!(q.needs_info());
        // Satırlar sonradan gelince (önbellekten) kurallar uygulanır.
        q.attach_info(rule_rows(&[("a", "X"), ("b", "Drake"), ("c", "Y")]));
        assert!(!q.needs_info());
        assert_eq!(q.upcoming_context(), ["t2"]);
        assert!(q.is_consistent());
    }

    #[test]
    fn attach_info_ignores_mismatched_rows() {
        let mut q = started(3, 0);
        q.attach_info(rule_rows(&[("a", "X")]));
        assert!(q.needs_info(), "uzunluk uymuyor: yok sayılır");
    }

    #[test]
    fn untraited_upcoming_lists_unknown_tracks_in_play_order() {
        let rows = rule_rows(&[("a", "X"), ("b", "Y"), ("c", "Z"), ("d", "W")]);
        let q = started_with(Rules::new(false, true, []), false, rows, 1, 1);
        let known = |u: &str| u == "t2";
        assert_eq!(q.untraited_upcoming(&known, 10), ["t3"]);
        assert_eq!(q.untraited_upcoming(&|_| false, 1), ["t2"]);
    }

    #[test]
    fn rules_are_a_pure_function_of_the_inputs() {
        let rows = rule_rows(&[
            ("a", "X"),
            ("b", "Drake"),
            ("c", "Y"),
            ("d", "Z"),
            ("e", "X"),
            ("f", "Y"),
        ]);
        let run = || {
            let mut q = started_with(
                Rules::new(true, false, ["drake"]),
                true,
                rows.clone(),
                0,
                77,
            );
            drain(&mut q)
        };
        assert_eq!(run(), run());
    }

    // ── gizlenen şarkılar (bkz. hidden.rs) ──

    fn hidden_list(entries: &[(&str, Option<&str>)]) -> Arc<HiddenList> {
        Arc::new(HiddenList::new(
            entries
                .iter()
                .enumerate()
                .map(|(i, (uri, ctx))| crate::engine::hidden::HiddenSong {
                    uri: (*uri).into(),
                    context: ctx.map(Into::into),
                    name: String::new(),
                    artists: String::new(),
                    hidden_at: i as i64,
                })
                .collect(),
        ))
    }

    fn hidden_queue(
        shuffle: bool,
        hidden: Arc<HiddenList>,
        context: Option<CollectionId>,
        index: usize,
        seed: u64,
    ) -> Queue {
        let rows = rule_rows(&[("a", "X"), ("b", "Y"), ("c", "Z"), ("d", "W"), ("e", "V")]);
        let mut q = Queue::new();
        q.set_shuffle(shuffle);
        q.set_hidden(hidden);
        q.restart(context, uris_of(&rows), Some(rows), index, seed, true)
            .unwrap();
        q
    }

    #[test]
    fn a_globally_hidden_song_is_skipped_in_order_and_in_shuffle() {
        for shuffle in [false, true] {
            let q = hidden_queue(shuffle, hidden_list(&[("t2", None)]), None, 0, 5);
            let mut left = sorted(q.upcoming_context());
            left.retain(|u| u == "t2");
            assert!(
                left.is_empty(),
                "karıştırma {shuffle}: gizli şarkı sırada olmamalı"
            );
            let mut q = q;
            let played = drain(&mut q);
            assert_eq!(played.len(), 3, "t0 çalıyor, t2 yok, kalan üçü: {played:?}");
            assert!(!played.contains(&"t2".to_string()));
        }
    }

    #[test]
    fn a_context_hidden_song_is_skipped_only_in_that_context() {
        let hidden = hidden_list(&[("t3", Some("spotify:playlist:aaa"))]);
        let here = hidden_queue(
            false,
            hidden.clone(),
            Some(CollectionId::Playlist("spotify:playlist:aaa".into())),
            0,
            1,
        );
        assert_eq!(here.upcoming_context(), ["t1", "t2", "t4"]);
        let elsewhere = hidden_queue(
            false,
            hidden.clone(),
            Some(CollectionId::Playlist("spotify:playlist:bbb".into())),
            0,
            1,
        );
        assert_eq!(elsewhere.upcoming_context(), ["t1", "t2", "t3", "t4"]);
        let none = hidden_queue(false, hidden, None, 0, 1);
        assert_eq!(
            none.upcoming_context(),
            ["t1", "t2", "t3", "t4"],
            "bağlamsız çalmada yalnız genel gizleme"
        );
    }

    #[test]
    fn a_hidden_song_the_user_picked_still_plays_and_the_queue_goes_on() {
        // Kullanıcı t2'yi seçti (restart'ın index'i): çalar; arkasından kalan gizlenmeyenler.
        let q = hidden_queue(
            false,
            hidden_list(&[("t2", None), ("t3", None)]),
            None,
            2,
            1,
        );
        assert_eq!(q.current_uri(), Some("t2"));
        assert_eq!(q.upcoming_context(), ["t4"]);
        // Sıraya elle eklenen gizli şarkı yine çalar (kullanıcı sırası süzülmez).
        let mut q = hidden_queue(false, hidden_list(&[("t1", None)]), None, 0, 1);
        q.enqueue(vec![item("t1")]);
        assert_eq!(q.next().as_deref(), Some("t1"));
        assert_eq!(q.next().as_deref(), Some("t2"));
    }

    #[test]
    fn previous_never_lands_on_a_hidden_song_when_started_mid_context() {
        let mut q = hidden_queue(false, hidden_list(&[("t1", None)]), None, 2, 1);
        // t2 çalıyor; bağlamda önceki t1 gizli → Önceki t0'a gider, t1'e değil.
        assert_eq!(q.previous().as_deref(), Some("t0"));
    }

    #[test]
    fn hiding_and_showing_while_playing_rebuilds_the_rest_only() {
        let mut q = hidden_queue(false, Arc::new(HiddenList::default()), None, 0, 1);
        assert_eq!(q.next().as_deref(), Some("t1"));
        assert_eq!(q.upcoming_context(), ["t2", "t3", "t4"]);
        assert!(q.set_hidden(hidden_list(&[("t3", None), ("t0", None)])));
        // Çalınmış kısım (t0, t1) yerinde; kalandan yalnız t3 düştü.
        assert_eq!(q.upcoming_context(), ["t2", "t4"]);
        assert_eq!(q.current_uri(), Some("t1"));
        assert!(
            !q.set_hidden(hidden_list(&[("t3", None), ("t0", None)])),
            "aynı küme değişiklik değil"
        );
        // Göster: t3 sıraya geri döner (henüz çalınmadığı için).
        assert!(q.set_hidden(Arc::new(HiddenList::default())));
        assert_eq!(q.upcoming_context(), ["t2", "t3", "t4"]);
        assert!(q.is_consistent());
    }

    #[test]
    fn hidden_songs_stay_out_of_repeat_all_laps_and_have_next_ignores_them() {
        for shuffle in [false, true] {
            let mut q = hidden_queue(
                shuffle,
                hidden_list(&[("t1", None), ("t4", None)]),
                None,
                0,
                9,
            );
            q.set_repeat(RepeatMode::All);
            for _ in 0..12 {
                let u = q.next().unwrap();
                assert!(
                    u == "t0" || u == "t2" || u == "t3",
                    "tur boyunca gizli şarkı yok: {u}"
                );
            }
        }
        // Hepsi gizliyse (çalan dışında) "tekrar hepsi" boş tur çevirmez.
        let mut q = hidden_queue(
            false,
            hidden_list(&[
                ("t1", None),
                ("t2", None),
                ("t3", None),
                ("t4", None),
                ("t0", None),
            ]),
            None,
            0,
            1,
        );
        q.set_repeat(RepeatMode::All);
        assert!(!q.has_next() && q.next().is_none());
    }

    #[test]
    fn restored_queue_is_only_rebuilt_when_it_holds_a_hidden_song() {
        let mut q = hidden_queue(true, Arc::new(HiddenList::default()), None, 0, 11);
        let order = q.upcoming_context();
        let gen_before = q.order_gen();
        // Gizli şarkı kalan sırada yoksa kayıtlı sıra olduğu gibi kalır.
        q.adopt_hidden(hidden_list(&[("tx-yok", None)]));
        q.refresh_if_needed();
        assert_eq!(
            (q.upcoming_context(), q.order_gen()),
            (order.clone(), gen_before)
        );
        // Varsa yalnız o düşer.
        let victim = order[1].clone();
        q.adopt_hidden(hidden_list(&[(victim.as_str(), None)]));
        q.refresh_if_needed();
        let after = q.upcoming_context();
        assert_eq!(after.len(), order.len() - 1);
        assert!(!after.contains(&victim));
    }

    // ── radyo: sonsuz bağlam (bkz. radio.rs) ──

    fn radio_rows(from: usize, to: usize) -> Vec<TrackRow> {
        (from..to)
            .map(|i| {
                rule_row(
                    i,
                    &format!("şarkı {i}"),
                    &format!("Sanatçı {}", i % 7),
                    180_000,
                )
            })
            .collect()
    }

    fn radio_queue(n: usize, shuffle: bool) -> Queue {
        let rows = Arc::new(radio_rows(0, n));
        let mut q = Queue::new();
        q.set_shuffle(shuffle);
        q.restart(None, uris_of(&rows), Some(rows), 0, 13, true)
            .unwrap();
        q.set_radio(Some(RadioInfo {
            seed: "spotify:track:s".into(),
            title: "Radyo".into(),
        }));
        q
    }

    #[test]
    fn extending_appends_new_songs_to_the_end_without_touching_what_was_played() {
        let mut q = radio_queue(6, false);
        q.next();
        q.next();
        let before_current = q.current_uri().map(str::to_owned);
        let history = q.has_previous();
        // t4 ve t5 zaten var, iki kez geçen t7 bir kez; üç yeni.
        let added = q.extend_context(
            radio_rows(4, 6)
                .into_iter()
                .chain(radio_rows(6, 9))
                .chain(radio_rows(7, 8))
                .collect(),
        );
        assert_eq!(added, 3);
        assert_eq!(q.context_uris().len(), 9);
        assert_eq!(q.upcoming_context(), ["t3", "t4", "t5", "t6", "t7", "t8"]);
        assert_eq!(
            (q.current_uri().map(str::to_owned), q.has_previous()),
            (before_current, history)
        );
        assert!(q.is_consistent());
        assert_eq!(q.extend_context(radio_rows(0, 9)), 0, "hepsi zaten var");
        assert_eq!(q.context_remaining(), 6);
    }

    #[test]
    fn extending_keeps_the_radio_and_a_new_context_drops_it() {
        let mut q = radio_queue(4, false);
        q.extend_context(radio_rows(4, 8));
        assert_eq!(q.radio().map(|r| r.title.as_str()), Some("Radyo"));
        assert_eq!(
            q.view(&|_| None).radio.map(|r| r.seed),
            Some("spotify:track:s".to_string())
        );
        q.restart(None, uris(3), None, 0, 1, true).unwrap();
        assert!(q.radio().is_none() && q.view(&|_| None).radio.is_none());
    }

    #[test]
    fn extended_songs_obey_the_rules_and_hidden_songs_and_shuffle_balances_them() {
        let mut q = radio_queue(5, true);
        q.set_rules(blocked(&["sanatçı 3"]));
        q.set_hidden(hidden_list(&[("t6", None)]));
        let added = q.extend_context(radio_rows(5, 25));
        assert_eq!(added, 20);
        let rest = q.upcoming_context();
        // Kara listedeki sanatçı (i % 7 == 3 → t10, t17, t24) ve gizli t6 sırada yok.
        for gone in ["t10", "t17", "t24", "t6"] {
            assert!(!rest.contains(&gone.to_string()), "{gone} sırada olmamalı");
        }
        // Yeni şarkıların hepsi bağlamda; sıra geçerli bir permütasyon.
        assert_eq!(q.context_uris().len(), 25);
        let mut uniq = rest.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), rest.len());
        assert!(q.is_consistent());
    }

    #[test]
    fn the_radio_context_is_capped() {
        let mut q = radio_queue(10, false);
        let added = q.extend_context(radio_rows(10, 5000));
        assert_eq!(added, crate::engine::radio::CTX_MAX - 10);
        assert_eq!(q.context_uris().len(), crate::engine::radio::CTX_MAX);
        assert_eq!(q.extend_context(radio_rows(5000, 5010)), 0);
    }

    #[test]
    fn extending_a_restored_queue_without_rows_still_works() {
        // Kaldığı yerden sürmede satırlar yok (info sonradan): uzatma bağlamı bozmaz.
        let mut q = Queue::new();
        q.restart(None, uris(4), None, 0, 1, true).unwrap();
        assert!(q.needs_info());
        assert_eq!(q.extend_context(radio_rows(4, 7)), 3);
        assert_eq!(q.upcoming_context(), ["t1", "t2", "t3", "t4", "t5", "t6"]);
        assert!(q.is_consistent());
    }

    // ── ölçüm: gizleme ve radyo uzatma maliyeti ──

    /// Risk: gizli şarkı süzgeci büyük bağlamda sırayı yavaşlatır. Çıta: 20 bin şarkılık karışık
    /// bağlamda 2 bin gizli şarkıyla `set_hidden` (sırayı baştan kurar) < 300 ms; radyonun 50'lik
    /// devamı 1000 şarkılık bağlamda < 20 ms. (`cargo test --release hidden_and_radio_costs -- --nocapture`)
    #[test]
    fn hidden_and_radio_costs_stay_small() {
        let n = 20_000;
        let rows = Arc::new(
            (0..n)
                .map(|i| rule_row(i, &format!("s{i}"), &format!("a{}", i % 400), 180_000))
                .collect::<Vec<_>>(),
        );
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.restart(
            Some(CollectionId::Playlist("spotify:playlist:big".into())),
            uris_of(&rows),
            Some(rows),
            0,
            7,
            true,
        )
        .unwrap();
        let entries: Vec<(String, Option<String>)> = (0..2000)
            .map(|i| (format!("t{}", i * 9 + 1), None))
            .collect();
        let hidden = hidden_list(
            &entries
                .iter()
                .map(|(u, c)| (u.as_str(), c.as_deref()))
                .collect::<Vec<_>>(),
        );
        let t0 = std::time::Instant::now();
        assert!(q.set_hidden(hidden));
        let hide_ms = t0.elapsed().as_secs_f64() * 1000.;
        assert_eq!(q.context_remaining(), n - 1 - 2000);

        let mut radio = radio_queue(950, true);
        let fresh = radio_rows(950, 1000);
        let t1 = std::time::Instant::now();
        assert_eq!(radio.extend_context(fresh), 50);
        let extend_ms = t1.elapsed().as_secs_f64() * 1000.;
        println!(
            "ölçüm: 20 bin şarkı, 2 bin gizli: set_hidden {hide_ms:.1} ms; radyo 50'lik devam (1000 şarkı): {extend_ms:.2} ms"
        );
        assert!(hide_ms < 300., "{hide_ms} ms");
        assert!(extend_ms < 20., "{extend_ms} ms");
    }
}
