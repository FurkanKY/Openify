//! Geçiş (crossfade) politikası: hangi geçişte, ne kadar bindirme olur.
//! Bindirmenin kendisi çıkışın yazarında (`output/writer.rs`); burası yalnız
//! kararı verir ve çıkışa `TransitionSpec` olarak iletir
//! (`OutputCtl::arm_transition`).
//!
//! Kurallar:
//! - Yalnız kendiliğinden geçişte (`Advance::Auto`, şarkı bitince). Kullanıcının
//!   Sonraki / Önceki / listeden seçmesi ve şarkı yüklemesi bindirmesizdir
//!   (zaten kuyruğu atar).
//! - Aynı albümün ardışık şarkıları bindirmesiz kalır (canlı albümler, kesintisiz
//!   konsept albümler boşluksuz bitişik olmalı).
//! - Tekrar "tek" (aynı şarkı yeniden başlıyor) bindirmesiz.
//! - Süre, ayardaki değerle iki şarkının yarısının en küçüğü; yazar ayrıca
//!   çözülmüş gerçek uzunluklara ve halkadaki boşluğa göre kısaltır.
//! - Arayüzün şarkı değişimi geçişin BAŞLANGICINDA (yeni şarkı ilk duyulduğu
//!   an; `Transition::start`), bkz. `handover.rs`.
//!
//! Üç kip (Ayarlar → Ses): Kapalı, Sabit (ayardaki saniye) ve AutoMix. AutoMix
//! aynı kuralları uygular (yalnız kendiliğinden geçiş, aynı albüm / tekrar-tek /
//! kullanıcı geçişi → bindirme yok); bindirmenin YERİ ve UZUNLUĞU şarkıların
//! çözümlemesinden gelir (`output/beat.rs`, `output/automix.rs`):
//! - giden şarkının çözümlemesi (son 20 sn zarfı, tempo, sessizlik) çalarken
//!   çıkışta üretilir, gelenin baş ucu önceki çalışlardan önbellekten (`beats`);
//! - burada yalnız ÖN plan yapılır (şarkı sonu payının derinliği için; önbellek
//!   yoksa sabit 5 sn); gerçek karar yazarda, şarkı bitince, canlı çözümlemeyle.

use super::{Advance, Cx, Effect, Playback};
use crate::engine::{
    TrackRow, fold_name,
    output::{
        AUTOMIX_FALLBACK_MS, Analysis, Curve, EdgeView, MAX_OVERLAP_MS, TransitionSpec, ring_frames,
    },
};

/// Bellekteki çözümleme defterinin en çok şarkısı (aşınca sıfırlanır; önbellek
/// durur, ihtiyaç olunca çalarken yeniden dolar).
const BOOK_MAX: usize = 4_000;

/// İki satır aynı albümden mi? Albüm adı boş değilse ve aynıysa; kapak
/// bilgisi iki yanda da varsa onun da aynı olması (aynı adlı farklı albümler,
/// "Greatest Hits"), yoksa ilk sanatçının aynı olması aranır.
pub(in crate::engine) fn same_album(a: &TrackRow, b: &TrackRow) -> bool {
    let (x, y) = (fold_name(&a.album), fold_name(&b.album));
    if x.is_empty() || x != y {
        return false;
    }
    if !a.image.is_empty() && !b.image.is_empty() {
        return a.image == b.image;
    }
    let first = |t: &TrackRow| fold_name(t.artists.split(", ").next().unwrap_or_default());
    first(a) == first(b)
}

/// Bir geçişin bindirmesi. `prev` / `next`: giden ve gelen şarkının satırı
/// (bilinmiyorsa `None`: albüm karşılaştırılamaz, süre sınırı uygulanmaz);
/// `same_track`: gelen, gidenin kendisi (tekrar "tek" ya da tek şarkılık sıranın
/// tekrarı); `crossfade_ms`: ayar; `how`: geçişi neyin başlattığı.
pub(in crate::engine) fn spec_for(
    prev: Option<&TrackRow>,
    next: Option<&TrackRow>,
    same_track: bool,
    crossfade_ms: u32,
    how: Advance,
) -> TransitionSpec {
    if crossfade_ms == 0 || how != Advance::Auto || same_track {
        return TransitionSpec::NONE;
    }
    if let (Some(p), Some(n)) = (prev, next)
        && same_album(p, n)
    {
        return TransitionSpec::NONE;
    }
    let mut ms = crossfade_ms.min(MAX_OVERLAP_MS);
    for track in [prev, next].into_iter().flatten() {
        if track.duration_ms > 0 {
            ms = ms.min(track.duration_ms / 2);
        }
    }
    TransitionSpec::from_ms(ms, Curve::EqualPower)
}

/// AutoMix: bir geçişin isteği. Kurallar `spec_for` ile aynı (yalnız kendiliğinden
/// geçiş; aynı albüm, tekrar-tek → yok; süre iki şarkının yarısını aşmaz). `out` / `inc`:
/// giden ve gelen şarkının önbellekteki çözümlemesi. Dönen ikinci değer gelenin baş ucu
/// (varsa; yazar tempo hizalaması için kullanır).
pub(in crate::engine) fn spec_for_auto(
    prev: Option<&TrackRow>,
    next: Option<&TrackRow>,
    same_track: bool,
    how: Advance,
    out: Option<&Analysis>,
    inc: Option<&Analysis>,
) -> (TransitionSpec, Option<EdgeView>) {
    if how != Advance::Auto || same_track {
        return (TransitionSpec::NONE, None);
    }
    if let (Some(p), Some(n)) = (prev, next)
        && same_album(p, n)
    {
        return (TransitionSpec::NONE, None);
    }
    let inc_view = inc.and_then(Analysis::head_view);
    // Giden şarkının ucu biliniyorsa önbellekten plan, yoksa sabit süre.
    let mut ms = match out.and_then(|a| a.plan(inc_view.as_ref())) {
        Some(plan) => (plan.overlap as u64 * 1000 / crate::engine::output::RATE as u64) as u32,
        None => AUTOMIX_FALLBACK_MS,
    };
    ms = ms.min(MAX_OVERLAP_MS);
    for track in [prev, next].into_iter().flatten() {
        if track.duration_ms > 0 {
            ms = ms.min(track.duration_ms / 2);
        }
    }
    (TransitionSpec::automix(ms), inc_view)
}

impl Playback {
    /// Ayarlardaki sabit geçiş süresi (`Command::SetCrossfade`; ms, 0 kapalı).
    /// Halka gerekirse büyütülür; şarkı sonu payı zaten açıksa derinliği yeni
    /// süreye göre yeniden belirlenir.
    pub fn set_crossfade(&mut self, cx: &mut Cx<'_>, ms: u32) {
        self.fixed_ms = ms.min(MAX_OVERLAP_MS);
        self.apply_crossfade(cx);
    }

    /// AutoMix kipini aç / kapat (`Command::SetAutoMix`). Açıkken halka en uzun
    /// bindirmeye göre boyutlanır (12 sn); çıkışın çözümleyicisi ve sessizlik
    /// atlama yalnız açıkken çalışır.
    pub fn set_automix(&mut self, cx: &mut Cx<'_>, on: bool) {
        self.automix = on;
        if !on {
            self.beats.clear();
        }
        self.apply_crossfade(cx);
    }

    /// Kip (sabit süre + AutoMix) değişti: halka ve pay derinliği buna göre.
    fn apply_crossfade(&mut self, cx: &mut Cx<'_>) {
        self.crossfade_ms = if self.automix {
            MAX_OVERLAP_MS
        } else {
            self.fixed_ms
        };
        if let Some(deck) = &self.deck {
            deck.out.reserve(ring_frames(self.crossfade_ms));
            deck.out.set_automix(self.automix);
        }
        if self.preload_armed {
            self.preview_transition(cx);
        }
    }

    /// Çıkıştan gelen yeni şarkı çözümlemelerini deftere alır ve önbelleğe yazdırır.
    pub(super) fn collect_analyses(&mut self, cx: &mut Cx<'_>) {
        let Some(deck) = &self.deck else { return };
        for (uri, analysis) in deck.out.take_analyses() {
            if self.beats.len() >= BOOK_MAX {
                self.beats.clear();
            }
            let book = match self.beats.get(&uri) {
                Some(old) => old.merged(&analysis),
                None => analysis,
            };
            self.beats.insert(uri.clone(), book);
            cx.fx.push(Effect::SaveAnalysis { uri, analysis });
        }
    }

    /// Bir geçişin isteği (kipe göre): sabit ya da AutoMix. Dönen ikinci değer
    /// AutoMix'te gelenin önbellekteki baş ucu.
    #[allow(clippy::too_many_arguments)]
    fn spec_now(
        &self,
        prev_uri: Option<&str>,
        prev: Option<&TrackRow>,
        next_uri: Option<&str>,
        next: Option<&TrackRow>,
        same_track: bool,
        how: Advance,
    ) -> (TransitionSpec, Option<EdgeView>) {
        if self.automix {
            let out = prev_uri.and_then(|u| self.beats.get(u));
            let inc = next_uri.and_then(|u| self.beats.get(u));
            spec_for_auto(prev, next, same_track, how, out, inc)
        } else {
            (
                spec_for(prev, next, same_track, self.crossfade_ms, how),
                None,
            )
        }
    }

    /// Çalan şarkının adresi ve satırı (sıradan, yoksa şarkı bilgilerinden).
    pub(super) fn current_track(cx: &Cx<'_>) -> Option<(String, Option<TrackRow>)> {
        let uri = cx.queue.current_uri()?.to_owned();
        let row = cx
            .queue
            .current_row()
            .or_else(|| cx.meta.get(&uri).cloned());
        Some((uri, row))
    }

    /// Sıradaki kendiliğinden geçiş için çıkışa bindirme isteğini ÖNCEDEN verir
    /// (şarkı sonu payı açılırken): payın derinliği bindirmeye göre belirlenir.
    /// Gerçek karar şarkı bitince `arm_transition`'da yeniden verilir.
    pub(super) fn preview_transition(&self, cx: &Cx<'_>) {
        let Some(deck) = &self.deck else { return };
        let (spec, incoming) = match (Self::current_track(cx), cx.queue.peek_next_auto()) {
            (Some((prev_uri, prev)), Some(next_uri)) => {
                // Gelenin satırı: sıranın kendi satırından ya da şarkı bilgilerinden
                // (bağlam satırları `meta`da olmayabilir, `view` ikisine de bakar).
                let view = cx.queue.view(&|u| cx.meta.get(u).cloned());
                let next = view
                    .upcoming
                    .iter()
                    .find(|e| e.uri == next_uri)
                    .and_then(|e| e.track.as_ref());
                self.spec_now(
                    Some(&prev_uri),
                    prev.as_ref(),
                    Some(&next_uri),
                    next,
                    false,
                    Advance::Auto,
                )
            }
            _ => (TransitionSpec::NONE, None),
        };
        deck.out.arm_automix(spec, incoming);
    }

    /// Çalara sıradaki şarkıyı yüklemeden HEMEN ÖNCE: bu geçişin bindirmesini
    /// çıkışa ver. `prev`: yüklemeden önce çalan şarkı (`current_track`),
    /// `next_uri`: yüklenecek; sıra zaten ilerledi, gelenin satırı oradan.
    pub(super) fn arm_transition(
        &self,
        cx: &Cx<'_>,
        prev: &Option<(String, Option<TrackRow>)>,
        next_uri: &str,
        how: Advance,
    ) {
        let Some(deck) = &self.deck else { return };
        let next = cx
            .queue
            .current_row()
            .or_else(|| cx.meta.get(next_uri).cloned());
        let same_track = prev.as_ref().is_some_and(|(uri, _)| uri == next_uri);
        let prev_row = prev.as_ref().and_then(|(_, row)| row.as_ref());
        let prev_uri = prev.as_ref().map(|(uri, _)| uri.as_str());
        let (spec, incoming) = self.spec_now(
            prev_uri,
            prev_row,
            Some(next_uri),
            next.as_ref(),
            same_track,
            how,
        );
        deck.out.arm_automix(spec, incoming);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uri: &str, album: &str, artists: &str, image: &str, duration_ms: u32) -> TrackRow {
        TrackRow {
            uri: uri.into(),
            name: uri.into(),
            artists: artists.into(),
            album: album.into(),
            duration_ms,
            image: image.into(),
            plays: None,
        }
    }

    fn a1() -> TrackRow {
        row("a1", "Album A", "Sanatçı", "img-a", 200_000)
    }

    #[test]
    fn off_means_no_overlap_whatever_the_transition() {
        let (p, n) = (a1(), row("b1", "Album B", "Başka", "img-b", 200_000));
        assert!(spec_for(Some(&p), Some(&n), false, 0, Advance::Auto).is_none());
    }

    #[test]
    fn natural_transition_between_albums_overlaps_for_the_setting() {
        let (p, n) = (a1(), row("b1", "Album B", "Başka", "img-b", 200_000));
        let spec = spec_for(Some(&p), Some(&n), false, 5_000, Advance::Auto);
        assert_eq!(spec, TransitionSpec::from_ms(5_000, Curve::EqualPower));
        assert_eq!(spec.overlap, 220_500);
    }

    #[test]
    fn consecutive_tracks_of_the_same_album_stay_gapless() {
        let (p, n) = (
            a1(),
            row("a2", "Album A", "Sanatçı, Konuk", "img-a", 200_000),
        );
        assert!(spec_for(Some(&p), Some(&n), false, 8_000, Advance::Auto).is_none());
        // Albüm adı aynı, kapak farklı: farklı albüm ("Greatest Hits").
        let other = row("c1", "album a", "Sanatçı", "img-other", 200_000);
        assert!(!spec_for(Some(&p), Some(&other), false, 8_000, Advance::Auto).is_none());
        // Kapak bilgisi yoksa ilk sanatçı karar verir.
        let (p, n) = (
            row("a1", "X", "Biri", "", 1),
            row("a2", "x", "Biri, Öteki", "", 1),
        );
        assert!(same_album(&p, &n));
        let n = row("a2", "x", "Başkası", "", 1);
        assert!(!same_album(&p, &n));
        // Albüm adı boşsa (bilinmiyor) aynı sayılmaz.
        assert!(!same_album(
            &row("a", "", "S", "", 1),
            &row("b", "", "S", "", 1)
        ));
    }

    #[test]
    fn user_driven_moves_never_overlap() {
        let (p, n) = (a1(), row("b1", "Album B", "Başka", "img-b", 200_000));
        for how in [Advance::Next, Advance::Previous] {
            assert!(
                spec_for(Some(&p), Some(&n), false, 8_000, how).is_none(),
                "{how:?}"
            );
        }
    }

    #[test]
    fn repeat_one_never_overlaps() {
        let p = a1();
        assert!(spec_for(Some(&p), Some(&p), true, 8_000, Advance::Auto).is_none());
    }

    #[test]
    fn overlap_never_exceeds_half_of_either_track() {
        let (p, n) = (
            row("a", "A", "S", "x", 30_000),
            row("b", "B", "T", "y", 200_000),
        );
        assert_eq!(
            spec_for(Some(&p), Some(&n), false, 12_000, Advance::Auto),
            TransitionSpec::from_ms(12_000, Curve::EqualPower)
        );
        let (p, n) = (
            row("a", "A", "S", "x", 20_000),
            row("b", "B", "T", "y", 200_000),
        );
        assert_eq!(
            spec_for(Some(&p), Some(&n), false, 12_000, Advance::Auto),
            TransitionSpec::from_ms(10_000, Curve::EqualPower)
        );
        let (p, n) = (
            row("a", "A", "S", "x", 200_000),
            row("b", "B", "T", "y", 6_000),
        );
        assert_eq!(
            spec_for(Some(&p), Some(&n), false, 12_000, Advance::Auto),
            TransitionSpec::from_ms(3_000, Curve::EqualPower)
        );
    }

    #[test]
    fn unknown_rows_still_overlap() {
        let spec = spec_for(None, None, false, 4_000, Advance::Auto);
        assert_eq!(spec, TransitionSpec::from_ms(4_000, Curve::EqualPower));
    }

    #[test]
    fn setting_is_capped_at_twelve_seconds() {
        assert_eq!(
            spec_for(None, None, false, 60_000, Advance::Auto),
            TransitionSpec::from_ms(12_000, Curve::EqualPower)
        );
    }

    // ── Çalma düzeyi: sinyal → çıkışa verilen istek ────────────────────

    use std::{collections::HashMap, sync::Arc};

    use super::super::{
        Gate, LinkFacts, PlayerSignal, TransportCmd,
        testkit::{self, Spy},
    };
    use crate::engine::{OutputCtl, RepeatMode, queue::Queue};

    fn uri(i: usize) -> String {
        format!("spotify:track:{i:022}")
    }

    /// Her şarkı: (albüm, kapak).
    fn rows(albums: &[(&str, &str)]) -> Arc<Vec<TrackRow>> {
        Arc::new(
            albums
                .iter()
                .enumerate()
                .map(|(i, (album, image))| row(&uri(i), album, "Sanatçı", image, 200_000))
                .collect(),
        )
    }

    struct Bench {
        pb: Playback,
        queue: Queue,
        meta: HashMap<String, TrackRow>,
        out: Arc<OutputCtl>,
        _spy: Spy,
    }

    impl Bench {
        fn new(albums: &[(&str, &str)], crossfade_ms: u32) -> Self {
            let (deck, spy, out) = testkit::fake_deck_for(crossfade_ms);
            let (tx, _events) = async_channel::unbounded();
            let (ptx, _view) = async_channel::unbounded();
            let mut pb = Playback::new(Gate::new(tx, ptx), 0);
            pb.deck = Some(deck);
            pb.crossfade_ms = crossfade_ms;
            let mut queue = Queue::new();
            let uris = (0..albums.len()).map(uri).collect();
            queue
                .restart(None, uris, Some(rows(albums)), 0, 1, true)
                .unwrap();
            let mut b = Self {
                pb,
                queue,
                meta: HashMap::new(),
                out,
                _spy: spy,
            };
            let first = b.queue.current_uri().unwrap().to_owned();
            b.run(|pb, cx| pb.begin(cx, first, 0));
            b
        }

        fn run<R>(&mut self, f: impl FnOnce(&mut Playback, &mut Cx<'_>) -> R) -> R {
            let mut fx = Vec::new();
            f(
                &mut self.pb,
                &mut Cx {
                    queue: &mut self.queue,
                    meta: &self.meta,
                    link: LinkFacts::default(),
                    fx: &mut fx,
                },
            )
        }

        fn signal(&mut self, signal: PlayerSignal) {
            self.run(|pb, cx| pb.on_signal(cx, signal));
        }

        fn armed(&self) -> TransitionSpec {
            self.out.armed_spec()
        }
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            self.pb.drop_handover();
        }
    }

    fn five() -> TransitionSpec {
        TransitionSpec::from_ms(5_000, Curve::EqualPower)
    }

    #[test]
    fn natural_end_arms_the_overlap_only_between_different_albums() {
        // Şarkı 0-1 albüm x, 2-3 albüm y.
        let mut b = Bench::new(&[("x", "ix"), ("x", "ix"), ("y", "iy"), ("y", "iy")], 5_000);
        b.signal(PlayerSignal::EndOfTrack); // 0 → 1: aynı albüm
        assert_eq!(b.queue.current_uri(), Some(uri(1).as_str()));
        assert!(
            b.armed().is_none(),
            "aynı albümün ardışık şarkısı boşluksuz kalmalı"
        );
        b.signal(PlayerSignal::EndOfTrack); // 1 → 2: albüm değişti
        assert_eq!(b.queue.current_uri(), Some(uri(2).as_str()));
        assert_eq!(b.armed(), five());
        b.signal(PlayerSignal::EndOfTrack); // 2 → 3: yine aynı albüm
        assert!(b.armed().is_none());
    }

    #[test]
    fn the_overlap_is_decided_ahead_of_time_when_the_tail_opens() {
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 5_000);
        assert!(b.armed().is_none());
        b.signal(PlayerSignal::PreloadDue);
        assert_eq!(
            b.armed(),
            five(),
            "pay açılırken istek önceden verilir (payın derinliği için)"
        );
        // Aynı albümde ön istek de bindirmesiz.
        let mut b = Bench::new(&[("x", "ix"), ("x", "ix")], 5_000);
        b.signal(PlayerSignal::PreloadDue);
        assert!(b.armed().is_none());
    }

    #[test]
    fn user_skips_never_overlap_even_after_the_overlap_was_announced() {
        for how in [TransportCmd::Next, TransportCmd::JumpTo(0)] {
            let mut b = Bench::new(&[("x", "ix"), ("y", "iy"), ("z", "iz")], 5_000);
            b.signal(PlayerSignal::PreloadDue);
            assert_eq!(b.armed(), five());
            b.run(|pb, cx| pb.transport(cx, how.clone()));
            assert!(
                b.armed().is_none(),
                "{how:?}: kullanıcı geçişi bindirmesiz olmalı"
            );
        }
        // Önceki: bir şarkı ilerledikten sonra geri dönmek de.
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy"), ("z", "iz")], 5_000);
        b.run(|pb, cx| pb.transport(cx, TransportCmd::Next));
        b.signal(PlayerSignal::PreloadDue);
        assert_eq!(b.armed(), five());
        b.run(|pb, cx| pb.transport(cx, TransportCmd::Previous));
        assert_eq!(b.queue.current_uri(), Some(uri(0).as_str()));
        assert!(b.armed().is_none(), "Önceki bindirmesiz olmalı");
        // Kullanıcı yeni bağlam / şarkı başlatınca da.
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 5_000);
        b.signal(PlayerSignal::PreloadDue);
        b.run(|pb, cx| pb.begin(cx, uri(1), 0));
        assert!(b.armed().is_none());
    }

    #[test]
    fn repeat_one_never_arms_an_overlap_through_the_signals() {
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 5_000);
        b.queue.set_repeat(RepeatMode::One);
        b.signal(PlayerSignal::PreloadDue);
        assert!(
            b.armed().is_none(),
            "tekrar tek: önyükleme de bindirme de yok"
        );
        b.signal(PlayerSignal::EndOfTrack);
        assert_eq!(
            b.queue.current_uri(),
            Some(uri(0).as_str()),
            "aynı şarkı yeniden"
        );
        assert!(b.armed().is_none());
    }

    #[test]
    fn single_track_repeat_all_never_overlaps_with_itself() {
        let mut b = Bench::new(&[("x", "ix")], 5_000);
        b.queue.set_repeat(RepeatMode::All);
        b.signal(PlayerSignal::EndOfTrack);
        assert!(b.armed().is_none());
    }

    #[test]
    fn off_means_off_across_albums() {
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 0);
        b.signal(PlayerSignal::PreloadDue);
        assert!(b.armed().is_none());
        b.signal(PlayerSignal::EndOfTrack);
        assert!(b.armed().is_none());
    }

    #[test]
    fn changing_the_setting_re_decides_an_open_tail_and_grows_the_ring_once() {
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 0);
        b.signal(PlayerSignal::PreloadDue);
        assert!(b.armed().is_none());
        // Pay açıkken ayardan 8 sn açıldı: istek hemen güncellenir, halka büyür.
        b.run(|pb, cx| pb.set_crossfade(cx, 8_000));
        assert_eq!(b.armed(), TransitionSpec::from_ms(8_000, Curve::EqualPower));
        assert!(b.out.has_second_slot(), "halka büyütülmedi");
        // Üst sınır 12 sn.
        b.run(|pb, cx| pb.set_crossfade(cx, 99_000));
        assert_eq!(b.pb.crossfade_ms, 12_000);
        // Kapatınca istek de gider (pay açık kalsa da bindirme yok).
        b.run(|pb, cx| pb.set_crossfade(cx, 0));
        assert!(b.armed().is_none());
    }

    #[test]
    fn crossfade_setting_is_kept_before_the_player_exists() {
        let (tx, _e) = async_channel::unbounded();
        let (ptx, _v) = async_channel::unbounded();
        let mut pb = Playback::new(Gate::new(tx, ptx), 0);
        let mut queue = Queue::new();
        let meta = HashMap::new();
        let mut fx = Vec::new();
        pb.set_crossfade(
            &mut Cx {
                queue: &mut queue,
                meta: &meta,
                link: LinkFacts::default(),
                fx: &mut fx,
            },
            5_000,
        );
        assert_eq!(
            pb.crossfade_ms, 5_000,
            "çalar yokken de saklanır (bağlanınca halka ona göre kurulur)"
        );
    }

    // ── AutoMix ─────────────────────────────────────────────────────────

    use crate::engine::output::{Analysis, HAS_HEAD, HAS_TAIL};

    fn auto_five() -> TransitionSpec {
        TransitionSpec::automix(5_000)
    }

    /// Giden şarkı: 7 sn'lik sönme (zarf: son saniyeler giderek alçalıyor).
    fn fading_tail() -> Analysis {
        let mut a = Analysis::EMPTY;
        a.flags = HAS_TAIL;
        a.tail_ms = 2_000;
        a.tail_env = [-14; crate::engine::output::ENV_SECS];
        for (k, v) in a.tail_env.iter_mut().enumerate().take(8) {
            *v = -14 - 4 * (8 - k as i8);
        }
        a
    }

    #[test]
    fn auto_follows_the_same_rules_as_the_fixed_crossfade() {
        let (p, n) = (a1(), row("b1", "Album B", "Başka", "img-b", 200_000));
        // Albümler arası kendiliğinden geçiş: AutoMix isteği, çözümleme yok: sabit 5 sn.
        let (spec, inc) = spec_for_auto(Some(&p), Some(&n), false, Advance::Auto, None, None);
        assert_eq!(spec, auto_five());
        assert!(spec.auto && inc.is_none());
        // Kullanıcı geçişleri, aynı albüm, tekrar-tek: bindirme yok.
        for how in [Advance::Next, Advance::Previous] {
            assert!(
                spec_for_auto(Some(&p), Some(&n), false, how, None, None)
                    .0
                    .is_none(),
                "{how:?}"
            );
        }
        let same = row("a2", "Album A", "Sanatçı", "img-a", 200_000);
        assert!(
            spec_for_auto(Some(&p), Some(&same), false, Advance::Auto, None, None)
                .0
                .is_none()
        );
        assert!(
            spec_for_auto(Some(&p), Some(&p), true, Advance::Auto, None, None)
                .0
                .is_none()
        );
        // Süre iki şarkının yarısını aşmaz.
        let short = row("s", "S", "T", "i", 6_000);
        let (spec, _) = spec_for_auto(Some(&p), Some(&short), false, Advance::Auto, None, None);
        assert_eq!(
            spec.overlap,
            TransitionSpec::from_ms(3_000, Curve::EqualPower).overlap
        );
    }

    #[test]
    fn auto_uses_the_cached_end_analysis_and_hands_over_the_incoming_head() {
        let (p, n) = (a1(), row("b1", "Album B", "Başka", "img-b", 200_000));
        let out = fading_tail();
        let (spec, inc) = spec_for_auto(Some(&p), Some(&n), false, Advance::Auto, Some(&out), None);
        assert_eq!(
            spec,
            TransitionSpec::automix(7_000),
            "7 sn'lik sönme: ön plan 7 sn"
        );
        assert!(inc.is_none(), "gelenin baş ucu bilinmiyor");
        // Gelenin baş ucu önbellekteyse yazara iletilir.
        let mut head = Analysis::EMPTY;
        head.flags = HAS_HEAD;
        head.head.bpm = 120.0;
        head.head.conf = 0.8;
        head.head_env = [-14; crate::engine::output::ENV_SECS];
        let (_, inc) = spec_for_auto(
            Some(&p),
            Some(&n),
            false,
            Advance::Auto,
            Some(&out),
            Some(&head),
        );
        assert_eq!(inc.map(|v| v.bpm), Some(120.0));
        // Yalnız baş ucu bilinen giden şarkıda (son uç yok) sabit 5 sn.
        let (spec, _) = spec_for_auto(Some(&p), Some(&n), false, Advance::Auto, Some(&head), None);
        assert_eq!(spec, auto_five());
    }

    #[test]
    fn automix_arms_through_the_signals_with_the_same_rules() {
        // Şarkı 0-1 albüm x, 2-3 albüm y.
        let mut b = Bench::new(&[("x", "ix"), ("x", "ix"), ("y", "iy"), ("y", "iy")], 0);
        b.run(|pb, cx| pb.set_automix(cx, true));
        assert_eq!(
            b.pb.crossfade_ms, 12_000,
            "AutoMix: halka en uzun bindirmeye göre"
        );
        b.signal(PlayerSignal::EndOfTrack); // 0 -> 1 aynı albüm
        assert!(b.armed().is_none());
        b.signal(PlayerSignal::EndOfTrack); // 1 -> 2 albüm değişti
        assert_eq!(b.armed(), auto_five());
        b.signal(PlayerSignal::EndOfTrack); // 2 -> 3 aynı albüm
        assert!(b.armed().is_none());
    }

    #[test]
    fn automix_user_skips_and_repeat_one_never_overlap() {
        for how in [TransportCmd::Next, TransportCmd::JumpTo(0)] {
            let mut b = Bench::new(&[("x", "ix"), ("y", "iy"), ("z", "iz")], 0);
            b.run(|pb, cx| pb.set_automix(cx, true));
            b.signal(PlayerSignal::PreloadDue);
            assert_eq!(b.armed(), auto_five());
            b.run(|pb, cx| pb.transport(cx, how.clone()));
            assert!(
                b.armed().is_none(),
                "{how:?}: kullanıcı geçişi bindirmesiz olmalı"
            );
        }
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 0);
        b.run(|pb, cx| pb.set_automix(cx, true));
        b.queue.set_repeat(RepeatMode::One);
        b.signal(PlayerSignal::PreloadDue);
        b.signal(PlayerSignal::EndOfTrack);
        assert!(b.armed().is_none(), "tekrar tek");
    }

    #[test]
    fn automix_plans_from_the_cache_when_the_tail_opens_and_switching_modes_re_decides() {
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 0);
        b.run(|pb, cx| pb.set_automix(cx, true));
        let current = b.queue.current_uri().unwrap().to_owned();
        b.pb.beats.insert(current, fading_tail());
        b.signal(PlayerSignal::PreloadDue);
        assert_eq!(
            b.armed(),
            TransitionSpec::automix(7_000),
            "önbellekten 7 sn"
        );
        // AutoMix açıkken sabit süre ayarı isteği değiştirmez.
        b.run(|pb, cx| pb.set_crossfade(cx, 8_000));
        assert!(b.armed().auto);
        // AutoMix kapanınca istek ayardaki sabit süreye döner, halka da.
        b.run(|pb, cx| pb.set_automix(cx, false));
        assert_eq!(b.armed(), TransitionSpec::from_ms(8_000, Curve::EqualPower));
        assert_eq!(b.pb.crossfade_ms, 8_000);
        assert!(
            b.pb.beats.is_empty(),
            "AutoMix kapanınca defter boşalır (bellek)"
        );
        b.run(|pb, cx| pb.set_crossfade(cx, 0));
        assert!(b.armed().is_none());
    }

    #[test]
    fn results_from_the_output_reach_the_book_and_the_save_effect() {
        let mut b = Bench::new(&[("x", "ix"), ("y", "iy")], 0);
        b.run(|pb, cx| pb.set_automix(cx, true));
        let mut a = Analysis::EMPTY;
        a.flags = HAS_HEAD;
        a.lead_ms = 321;
        // Çalar iş parçacığının yayını: motor bir sonraki sinyalde alır.
        b.out.publish_analysis_for_test("spotify:track:x", a);
        let mut fx = Vec::new();
        {
            let mut cx = Cx {
                queue: &mut b.queue,
                meta: &b.meta,
                link: LinkFacts::default(),
                fx: &mut fx,
            };
            b.pb.collect_analyses(&mut cx);
        }
        assert_eq!(
            b.pb.beats.get("spotify:track:x").map(|a| a.lead_ms),
            Some(321)
        );
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::SaveAnalysis { uri, .. } if uri == "spotify:track:x"))
        );
        // İkinci yayın eskisiyle birleşir (baş uç korunur, yeni son uç eklenir).
        let mut t = Analysis::EMPTY;
        t.flags = HAS_TAIL;
        t.tail_ms = 99;
        b.out.publish_analysis_for_test("spotify:track:x", t);
        {
            let mut cx = Cx {
                queue: &mut b.queue,
                meta: &b.meta,
                link: LinkFacts::default(),
                fx: &mut fx,
            };
            b.pb.collect_analyses(&mut cx);
        }
        let book = b.pb.beats.get("spotify:track:x").unwrap();
        assert_eq!(
            (book.flags, book.lead_ms, book.tail_ms),
            (HAS_HEAD | HAS_TAIL, 321, 99)
        );
    }
}
