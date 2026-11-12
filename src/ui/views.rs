//! Pencerenin önbelleğe alınan parçaları.
//!
//! gpui bir parçayı önceki kareden yalnız o parça ayrı bir görünümse (view)
//! yeniden kullanabiliyor. Raf, liste ve hap bu yüzden ince görünümlerde:
//! çizimi yine `Shell`'in fonksiyonları yapar, görünüm onları yalnız kendi
//! adına çağırır (dinleyiciler `Shell`'e bağlı kalır). Çalarken her karede
//! yalnız değişen parça (hap ya da ekolayzer) yeniden kurulur; gerisi önceki
//! kareden olduğu gibi tekrar oynatılır.
//!
//! Görünümler `Shell`'in durumunu okuyor ama gpui bunu kendisi izlemiyor:
//! her biri `Shell`'i gözler, `Shell` her yenilendiğinde o da yenilenir
//! (önce doğruluk). Çalarken saniyede onlarca kez değişen şeyler (ilerleme,
//! ekolayzer) bu yüzden `Shell`'i değil yalnız kendi görünümünü yeniler.

use std::time::Instant;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, Subscription, WeakEntity, Window, div,
    prelude::*,
};

use super::{Shell, gallery, library::{eq_levels, equalizer_bars}};

/// `Shell`'in görünümleri; hepsi kökün doğrudan çocuğu (iç içe önbellekli
/// görünüm olmasın: kirlenen dış görünüm içtekileri de yeniden çizdirir).
pub struct Views {
    pub shelf: Entity<ShelfView>,
    pub list: Entity<ListView>,
    pub pill: Entity<PillView>,
    pub search: Entity<SearchView>,
    pub equalizer: Entity<EqualizerLayer>,
    /// Sağdaki yüzen panel (sıra, sözler) ve tam ekran (bkz. `now_playing`).
    pub side: Entity<super::now_playing::SideView>,
    pub full: Entity<super::now_playing::FullView>,
    /// Başlık çubuğundaki geri / ileri / ana sayfa (Adım 12).
    pub nav: Entity<NavView>,
}

impl Views {
    pub fn new(shell: &Entity<Shell>, window: &mut Window, cx: &mut App) -> Self {
        let shelf = cx.new(|cx| ShelfView { shell: shell.downgrade(), _follow: follow(shell, cx) });
        let list = cx.new(|cx| ListView { shell: shell.downgrade(), _follow: follow(shell, cx), home_tick: None });
        let pill = cx.new(|cx| PillView { shell: shell.downgrade(), _follow: follow(shell, cx) });
        let search = cx.new(|cx| SearchView { shell: shell.downgrade(), _follow: follow(shell, cx) });
        let equalizer = cx.new(|cx| EqualizerLayer {
            shell: shell.downgrade(),
            // Liste kendi kendine de yenilenir (kaydırma, üzerine gelme):
            // çalan satır yer değiştirmiş olabilir, katman da yenilensin.
            // Duraklatılmışken katman kendiliğinden hiç yenilenmiyor.
            _follow: [follow(shell, cx), follow(&list, cx)],
            // Pencere öne gelince ya da arkaya düşünce hız değişir.
            _activation: cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            waiting: false,
        });
        let side = super::now_playing::SideView::new(shell, cx);
        let full = super::now_playing::FullView::new(shell, cx);
        let nav = cx.new(|cx| NavView { shell: shell.downgrade(), _follow: follow(shell, cx) });
        Self { shelf, list, pill, search, equalizer, side, full, nav }
    }
}

/// Başlık çubuğundaki gezinme düğmeleri: arama gibi pencere düğmesi isabet
/// alanı yok, önceki kareden tekrar oynatılabilir (kök her karede kuruluyor;
/// ekolayzer akarken düğmeler yeniden kurulmasın).
pub struct NavView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
}

impl Render for NavView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_part(&self.shell, window, cx, |shell, window, cx| shell.render_nav_buttons(window, cx))
    }
}

/// `target` her yenilendiğinde görünümü de yenile.
fn follow<T: 'static, V: 'static>(target: &Entity<T>, cx: &mut Context<V>) -> Subscription {
    cx.observe(target, |_, _, cx| cx.notify())
}

/// Parçayı `Shell` adına çizer. Parçanın kökü görünümün bütün alanını
/// kaplar: önbellekli görünümün boyu içeriğinden değil, yerini tutan
/// kutudan gelir (bkz. `Shell::render`).
fn render_part<V: 'static>(
    shell: &WeakEntity<Shell>,
    window: &mut Window,
    cx: &mut Context<V>,
    part: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) -> AnyElement,
) -> AnyElement {
    shell
        .update(cx, |shell, cx| {
            let element = part(shell, window, cx);
            // Resimler artık bu görünümün çiziminde isteniyor.
            shell.flush_image_requests(cx);
            // Bu parçada yeni inen bir kapak hâlâ açılıyorsa yalnız bu görünüm
            // bir kare daha çizilir (geçiş bitene kadar).
            if shell.images.take_fading() {
                window.request_animation_frame();
            }
            element
        })
        .map(|element| div().size_full().flex().flex_col().child(element).into_any_element())
        .unwrap_or_else(|_| div().into_any_element())
}

/// Üstteki kapak rafı (liste kayınca şerit).
pub struct ShelfView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
}

impl Render for ShelfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_part(&self.shell, window, cx, |shell, window, cx| shell.render_shelf(window, cx))
    }
}

/// Açık listenin başlığı ve şarkıları; "Tümü" açıkken kapak ızgarası (ya da
/// vitrin).
pub struct ListView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
    /// Ana sayfada çalarken saniyede bir yenileme (ilerleme çubuğu; bkz.
    /// `Shell::home_hero_ticks`).
    home_tick: Option<gpui::Task<()>>,
}

impl Render for ListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Ana sayfadaki kahraman kartın ilerleme çubuğu: yalnız ana sayfa
        // görünürken ve çalarken, saniyede bir (yalnız bu görünüm yenilenir).
        let ticks = self.shell.upgrade().is_some_and(|shell| shell.read(cx).home_hero_ticks());
        if !ticks {
            self.home_tick = None;
        } else if self.home_tick.is_none() {
            self.home_tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let _ = this.update(cx, |view, cx| {
                    view.home_tick = None;
                    cx.notify();
                });
            }));
        }
        render_part(&self.shell, window, cx, |shell, window, cx| {
            // Liste gerçekten yeniden kuruluyor: çalan satır görünürse yeri
            // bu çizimde yeniden ölçülür, görünmezse ekolayzer çizilmez.
            // (Listenin tekerlek hareketi kökte ilerletilir, bkz.
            // `Shell::advance_list_glide`: raf da aynı adımı okusun.)
            shell.playing_cell.set(None);
            shell.list_viewport.set(None);
            if gallery::enabled() {
                shell.render_gallery(window, cx)
            } else if shell.all.open {
                // Izgaranın tekerlek hareketi yalnız bu görünümü ilgilendirir.
                if shell.tick_all_glide() {
                    window.request_animation_frame();
                }
                shell.render_all(window, cx)
            } else if shell.page_open() {
                // Sayfa (sanatçı, albüm, ana sayfa, arama; bkz. `pages`).
                if shell.tick_page_glide() {
                    window.request_animation_frame();
                }
                shell.render_page(window, cx)
            } else {
                shell.render_collection(window, cx)
            }
        })
    }
}

/// Alttaki yüzen çalar hapı.
pub struct PillView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
}

impl Render for PillView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_part(&self.shell, window, cx, |shell, window, cx| {
            // Ekrana çizilecek olanın özeti, çizimden önce: zamanlayıcı yalnız
            // bu değişince hapı yeniler (sonra alınsaydı aradaki değişim
            // kaçabilirdi).
            shell.bar_signature = Some(shell.bar_signature_at(Instant::now()));
            shell.render_pill(window, cx)
        })
    }
}

/// Başlık çubuğunun ortasındaki arama alanı.
///
/// Başlık çubuğu önbelleğe alınamıyor (pencere düğmeleri, bkz. `Shell::render`)
/// ve kök her karede kurulduğu için ekolayzerin her karesinde onunla birlikte
/// yeniden çiziliyordu. Arama alanının pencere düğmesi isabet alanı yok:
/// önceki kareden tekrar oynatılabilir.
pub struct SearchView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_part(&self.shell, window, cx, |shell, window, cx| shell.render_search(window, cx))
    }
}

/// Ekolayzerin kare hızı: pencere öndeyken 30, arkadayken 20 (çalarken
/// işlemci çıtası). Geliştirme kancası
/// `OPENIFY_EQ_FPS=N` hızı zorlar, 0 ekranın hızı. (Kanca bir kez okunur.)
fn eq_fps(window: &Window) -> u32 {
    static FORCED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    let forced = *FORCED.get_or_init(|| std::env::var("OPENIFY_EQ_FPS").ok().and_then(|v| v.parse().ok()));
    forced.unwrap_or(if window.is_window_active() { 30 } else { 20 })
}

/// `fps` hızında akan ekolayzerin bir sonraki karesi: ekranın her
/// yenilenmesinde (çizmeden) bakılır, sıra gelince yalnız katman uyarılır.
/// Böylece kareler ekranın yenilenmesine hizalı kalır. Eşik sürenin %85'i:
/// yenilenme anlarındaki küçük oynamalar kare atlatmasın.
fn wait_for_tick(window: &Window, view: WeakEntity<EqualizerLayer>, since: Instant, fps: u32) {
    let due = std::time::Duration::from_secs_f32(0.85 / fps as f32);
    window.on_next_frame(move |window, cx| {
        if since.elapsed() >= due {
            let _ = view.update(cx, |layer, cx| {
                layer.waiting = false;
                cx.notify();
            });
        } else {
            wait_for_tick(window, view, since, fps);
        }
    });
}

/// Çalan şarkının yanındaki ekolayzer, listenin üstünde ayrı bir katman.
///
/// Çalarken her karede yeniden çizilir; liste ve geri kalan pencere önceki
/// kareden kullanılır. Satırdaki hücrenin ve listenin ekrandaki yeri,
/// listenin kendi çiziminde ölçülür (`Shell::playing_cell`). Katman listeden
/// sonra geldiği için aynı karede güncel yeri okur: kaydırırken geride
/// kalmaz. Satırların göründüğü alanla kırpılır (rafın ve başlığın altına
/// taşmaz); hap katmandan sonra çizildiği için hapın altına giren satırın
/// ekolayzeri hapla örtülür. Fareyi hiç yakalamaz (kimlik ve dinleyici
/// yok), tıklamalar alttaki listeye gider.
pub struct EqualizerLayer {
    shell: WeakEntity<Shell>,
    _follow: [Subscription; 2],
    _activation: Subscription,
    /// Bir sonraki kare zaten bekleniyor (iki bekleyiş birden kurulup hızı
    /// katlamasın).
    waiting: bool,
}

/// Ekolayzer çubuklarının bu karedeki yükseklikleri (0–1): çalarken o an
/// DUYULAN sesin bas / orta / tiz enerjisi (motorun ses dokunuşu, gecikmesi
/// hesaba katılmış; bir kare sonrası için sorulur). Ses henüz gelmediyse
/// çubuklar alçakta bekler. Duraklatınca merdiven duruşu. Sahte çalmada
/// (`OPENIFY_TEST_FAKE_PLAY`, ses yok) saatten hesaplanan hareket sürer:
/// kare ölçümleri ve ekran sınamaları gerçek sese bağlı kalmasın.
fn eq_levels_now(shell: &Shell, playing: bool) -> [f32; 3] {
    static FAKE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !playing || *FAKE.get_or_init(|| std::env::var_os("OPENIFY_TEST_FAKE_PLAY").is_some()) {
        return eq_levels(playing);
    }
    shell
        .engine
        .levels()
        .read(Instant::now() + std::time::Duration::from_millis(16))
        .unwrap_or([0.; 3])
}

impl Render for EqualizerLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
        let shell = shell.read(cx);
        let (Some(cell), Some(clip)) = (shell.playing_cell.get(), shell.list_viewport.get()) else {
            return div().into_any_element();
        };
        let playing = shell.state.is_playing();
        // Windows'ta animasyonlar kapalıysa çubuklar akmaz (karar 6 Kasım):
        // duraklatılmış gibi merdiven duruşunda kalır; çalıyor olduğu hapta
        // ve rafta görünür. Önce bu ayarda da ekranın hızında akıyordu.
        let moving = playing && super::theme::animations_enabled();
        let color = shell.accent().text;
        // Palet açıkken liste örtülü: bütün pencere (palet dahil) her karede
        // yeniden kurulmasın diye ekolayzer o sırada durur.
        if moving && !shell.palette.open && !shell.now.full.open {
            match eq_fps(window) {
                0 => window.request_animation_frame(),
                fps if !self.waiting => {
                    self.waiting = true;
                    wait_for_tick(window, cx.entity().downgrade(), Instant::now(), fps);
                }
                _ => {}
            }
        }
        // Katman pencerenin sol üst köşesinde: ölçülen yerler doğrudan konum.
        div().size_full().child(
            // Liste alanının dışına taşmasın: yarısı sütun başlıklarının
            // altında kalan satırın ekolayzeri de kırpılır.
            div()
                .absolute()
                .left(clip.origin.x)
                .top(clip.origin.y)
                .w(clip.size.width)
                .h(clip.size.height)
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .left(cell.origin.x - clip.origin.x)
                        .top(cell.origin.y - clip.origin.y)
                        .w(cell.size.width)
                        .h(cell.size.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(equalizer_bars(color, eq_levels_now(shell, moving))),
                ),
        )
        .into_any_element()
    }
}
