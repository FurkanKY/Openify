//! Fare tekerleğiyle yumuşak kaydırma.
//!
//! gpui'de Windows tekerleği her çentikte içeriği bir anda üç satır (~80 px)
//! atlatıyordu: göz satırları kaybediyor, kaydırma "takılıyor" gibi
//! görünüyordu. Burada çentik bir hedef mesafe olur ve içerik oraya üstel
//! bir izleyiciyle (her karede kalan yolun sabit bir oranı) kayar: art arda
//! çevrilen çentikler birikir, yön değişince birikim sıfırlanır.
//!
//! Dokunmatik yüzey ve ince adımlı tekerlek zaten küçük, sık adımlar
//! gönderiyor: onlar parmağı bire bir izlesin diye hemen uygulanır (üstüne
//! animasyon gecikme gibi hissettirir). Windows animasyonları kapalıysa da
//! hemen uygulanır.
//!
//! Durum `Cell`'lerde: dinleyici ve çizim `&self` ile kullanır.

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use gpui::{Pixels, ScrollDelta};

/// İzleyicinin zaman sabiti: kalan yolun ~%63'ü bu sürede alınır. Bir çentik
/// (~80 px) ~150 ms'de yerine oturur.
const TAU_MS: f32 = 35.;
/// Kalan yol bundan kısaysa son adımda tamamlanır.
const SNAP_PX: f32 = 0.5;
/// Kareler arası süre bu aralığa sıkıştırılır: ilk karede ya da takılan bir
/// karede tek adımda fazla (ya da hiç) yol alınmasın.
const DT_MIN: Duration = Duration::from_millis(4);
const DT_MAX: Duration = Duration::from_millis(40);
/// Kesirli (dokunmatik yüzey) bir adımdan sonra bu süre boyunca gelen tam
/// çentikler de hemen uygulanır: yüzey arada bir tam çentik gönderebiliyor.
const PRECISE_HOLD: Duration = Duration::from_millis(150);
/// Sayfa sayfa kaydırma ayarında bir çentik görünen alanın bu kadarı.
const PAGE_FRACTION: f32 = 0.9;
/// Biriken yol görünen alanın en çok bu katı (hızlı çevrilen tekerlek
/// içeriği sayfalarca öteye fırlatmasın).
const MAX_BACKLOG: f32 = 2.;

/// Tekerlek olayının ne yapılacağı (px, + aşağı).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wheel {
    /// Yumuşak kayarak git.
    Glide(f32),
    /// Hemen uygula (dokunmatik yüzey, animasyonlar kapalı).
    Now(f32),
}

#[derive(Default)]
pub struct SmoothScroll {
    /// Alınacak yol (px, + aşağı).
    remaining: Cell<f32>,
    /// Son adımın (ya da hareketi başlatan olayın) zamanı.
    last_tick: Cell<Option<Instant>>,
    /// Son adımdan sonraki konum: başka biri (klavye, kaydırma çubuğu,
    /// gpui'nin sınıra kırpması) konumu değiştirdiyse hareket bırakılır.
    last_position: Cell<Option<f32>>,
    /// Bu ana kadar tam çentikler de hemen uygulanır (bkz. `PRECISE_HOLD`).
    precise_until: Cell<Option<Instant>>,
    /// Kareler arası sürenin yumuşatılmış tahmini (ms; 0 = henüz yok).
    /// Çizimler ekrana çıkış anına göre bir ms kadar oynuyor (180 Hz'de 4,4
    /// ile 6,2 ms arası); ölçülen süreyle adım atılınca adımlar karede
    /// ±%20 sekiyordu. Ekran sabit aralıkla gösterdiği için tahminle adım
    /// atılır.
    frame_ms: Cell<f32>,
    /// Hareket yeni başladı: ilk adımın süresi olaydan bu yana geçen süre
    /// değil, bir karelik süre.
    fresh: Cell<bool>,
}

impl SmoothScroll {
    /// Hareket sürüyor mu?
    pub fn is_active(&self) -> bool {
        self.remaining.get() != 0.
    }

    /// Hareketi bırak (konum olduğu yerde kalır).
    pub fn cancel(&self) {
        self.remaining.set(0.);
        self.last_tick.set(None);
        self.last_position.set(None);
        self.fresh.set(false);
    }

    /// Tekerlek olayını sınıflandır. `viewport`: kaydırılan alanın yüksekliği.
    /// Dikey bileşen yoksa (Shift ile yatay) `None`.
    pub fn classify(&self, delta: ScrollDelta, line_height: Pixels, viewport: f32, now: Instant) -> Option<Wheel> {
        self.classify_with(delta, line_height, viewport, now, wheel_scroll_lines(), super::theme::animations_enabled())
    }

    fn classify_with(
        &self,
        delta: ScrollDelta,
        line_height: Pixels,
        viewport: f32,
        now: Instant,
        lines_per_notch: f32,
        animate: bool,
    ) -> Option<Wheel> {
        let lines = match delta {
            // Piksel adımı (macOS / Linux dokunmatik yüzeyi): bire bir.
            ScrollDelta::Pixels(p) => {
                let d = -f32::from(p.y);
                if d == 0. {
                    return None;
                }
                self.precise_until.set(Some(now + PRECISE_HOLD));
                return Some(Wheel::Now(d));
            }
            ScrollDelta::Lines(l) => l.y,
        };
        if lines == 0. || lines_per_notch <= 0. {
            return None;
        }
        let notches = lines / lines_per_notch;
        let whole = notches.abs() >= 0.999 && (notches - notches.round()).abs() < 1e-3;
        // Sayfa sayfa kaydırma ayarı (Windows'ta satır sayısı yerine çok
        // büyük bir değer): çentik başına bir sayfaya yakın.
        let d = if lines_per_notch >= PAGE_LINES {
            -notches * PAGE_FRACTION * viewport.max(0.)
        } else {
            -lines * f32::from(line_height)
        };
        if !whole {
            self.precise_until.set(Some(now + PRECISE_HOLD));
            return Some(Wheel::Now(d));
        }
        let precise = self.precise_until.get().is_some_and(|until| now < until);
        Some(if precise || !animate { Wheel::Now(d) } else { Wheel::Glide(d) })
    }

    /// Yola `delta` ekle (aynı yönde birikir, ters yönde baştan başlar).
    pub fn push(&self, delta: f32, viewport: f32, now: Instant) {
        let left = self.remaining.get();
        let base = if left != 0. && left.signum() != delta.signum() { 0. } else { left };
        let cap = (MAX_BACKLOG * viewport).max(delta.abs());
        self.remaining.set((base + delta).clamp(-cap, cap));
        if self.last_tick.get().is_none() {
            self.last_tick.set(Some(now));
            self.fresh.set(true);
        }
    }

    /// Bu adımın süresi (ms): ölçülen süre, kare aralığı tahminiyle
    /// yumuşatılmış. Takılan bir karede (tahminin ~2 katı) gerçek süre
    /// kullanılır, hareket geride kalmasın.
    fn step_ms(&self, measured: Duration) -> f32 {
        let raw = measured.clamp(DT_MIN, DT_MAX).as_secs_f32() * 1000.;
        let estimate = self.frame_ms.get();
        if self.fresh.replace(false) {
            return if estimate > 0. { estimate } else { raw.min(1000. / 60.) };
        }
        if estimate <= 0. {
            self.frame_ms.set(raw);
            return raw;
        }
        if raw > 1.75 * estimate {
            // Takılma ya da daha yavaş bir ekran: tahmin yavaşça uyar.
            self.frame_ms.set(estimate + 0.1 * (raw - estimate));
            return raw;
        }
        let smoothed = estimate + 0.15 * (raw - estimate);
        self.frame_ms.set(smoothed);
        smoothed
    }

    /// Bir karelik adım. `position`: şu anki konum (px); `apply` adımı
    /// uygular ve gerçekten alınan yolu döndürür (sınıra gelince daha az).
    /// Hareket sürüyorsa `true`: çağıran bir sonraki kareyi istemeli.
    pub fn tick(&self, now: Instant, position: f32, apply: impl FnOnce(f32) -> f32) -> bool {
        let left = self.remaining.get();
        if left == 0. {
            return false;
        }
        // Konumu başka biri değiştirdiyse onunla yarışma.
        if self.last_position.get().is_some_and(|p| (p - position).abs() > SNAP_PX) {
            self.cancel();
            return false;
        }
        let measured = self.last_tick.get().map_or(DT_MIN, |t| now.saturating_duration_since(t));
        let dt_ms = self.step_ms(measured);
        let mut step = left * (1. - (-dt_ms / TAU_MS).exp());
        if (left - step).abs() <= SNAP_PX {
            step = left;
        }
        let moved = apply(step);
        // Sınıra gelindi: kalan yol boşa gider.
        if (moved - step).abs() > 0.01 {
            self.cancel();
            return false;
        }
        let left = left - step;
        if left == 0. {
            self.cancel();
            return false;
        }
        self.remaining.set(left);
        self.last_tick.set(Some(now));
        self.last_position.set(Some(position + moved));
        true
    }
}

/// `SPI_GETWHEELSCROLLLINES` bundan büyükse sayfa sayfa kaydırma demek
/// (Windows `WHEEL_PAGESCROLL` = u32::MAX verir).
const PAGE_LINES: f32 = 1000.;

/// Bir çentiğin kaç satır kaydırdığı (Windows ayarı; gpui de bunu kullanıyor).
/// Her olayda okunur: ayar değişince hemen geçerli olsun (çağrı ucuz).
#[cfg(windows)]
fn wheel_scroll_lines() -> f32 {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETWHEELSCROLLLINES, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut lines: u32 = 3;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some(&mut lines as *mut u32 as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() { 3. } else { lines as f32 }
}

#[cfg(not(windows))]
fn wheel_scroll_lines() -> f32 {
    3.
}

/// Kaydırılan alanın üstüne bindirilen tekerlek yakalayıcısı (yakalama
/// aşamasında, gpui'nin kendi kaydırmasından önce). Tek kural: tekerlek
/// yalnız, imlecin altındaki en üst öğe bu alansa işlenir
/// (`Hitbox::should_handle_scroll`). Üstüne binen her katman (sağdaki sıra /
/// söz paneli, eşitleme paneli, menü, ad kutusu, bildirim) `.occlude()` ile
/// tekerleği de örter: olay yakalanmadan altına geçer, o katmanın kendi
/// kaydırması çalışır. Hap ve bildirim şeridi `block_mouse_except_scroll`:
/// onların altındaki satırlar tekerlekle kaymaya devam eder. `on_wheel`
/// (olay, alanın boyu, çizen görünüm) işlediyse `true` döner, olay orada biter.
pub fn wheel_zone(
    on_wheel: impl Fn(&gpui::ScrollWheelEvent, gpui::Bounds<Pixels>, gpui::EntityId, &mut gpui::Window, &mut gpui::App) -> bool + 'static,
) -> gpui::Canvas<gpui::Hitbox> {
    use gpui::{DispatchPhase, HitboxBehavior, ScrollWheelEvent, Styled};
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, _| {
            let view = window.current_view();
            window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                if on_wheel(ev, bounds, view, window, cx) {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    const LINE: f32 = 26.;

    fn lines(y: f32) -> ScrollDelta {
        ScrollDelta::Lines(point(0., y))
    }

    #[test]
    fn whole_notches_glide_fractional_steps_apply_now() {
        let s = SmoothScroll::default();
        let t = Instant::now();
        // Bir çentik aşağı (gpui: -3 satır).
        assert_eq!(s.classify_with(lines(-3.), px(LINE), 600., t, 3., true), Some(Wheel::Glide(3. * LINE)));
        // Dokunmatik yüzey: kesirli, hemen ve bire bir.
        assert_eq!(s.classify_with(lines(0.3), px(LINE), 600., t, 3., true), Some(Wheel::Now(-0.3 * LINE)));
        // Hemen ardından gelen tam çentik de hemen uygulanır.
        let soon = t + Duration::from_millis(50);
        assert_eq!(s.classify_with(lines(-3.), px(LINE), 600., soon, 3., true), Some(Wheel::Now(3. * LINE)));
        let later = t + Duration::from_millis(400);
        assert_eq!(s.classify_with(lines(-3.), px(LINE), 600., later, 3., true), Some(Wheel::Glide(3. * LINE)));
        // Animasyonlar kapalı: hemen.
        assert_eq!(s.classify_with(lines(-6.), px(LINE), 600., later, 3., false), Some(Wheel::Now(6. * LINE)));
        // Yatay (Shift) ya da 0 satır ayarı: dokunulmaz.
        assert_eq!(s.classify_with(lines(0.), px(LINE), 600., later, 3., true), None);
    }

    #[test]
    fn page_setting_scrolls_most_of_a_page() {
        let s = SmoothScroll::default();
        let per = u32::MAX as f32;
        let w = s.classify_with(lines(-per), px(LINE), 500., Instant::now(), per, true);
        assert_eq!(w, Some(Wheel::Glide(450.)));
    }

    /// Hareketi sonuna kadar oynatır; her karedeki adımları döndürür.
    fn run(s: &SmoothScroll, start: Instant, frame_ms: u64, limit: Option<f32>) -> (f32, Vec<f32>) {
        let mut pos = 0.;
        let mut steps = Vec::new();
        let mut now = start;
        for _ in 0..1000 {
            now += Duration::from_millis(frame_ms);
            let mut taken = 0.;
            let more = s.tick(now, pos, |d| {
                let next = match limit {
                    Some(max) => (pos + d).clamp(0., max),
                    None => pos + d,
                };
                taken = next - pos;
                taken
            });
            pos += taken;
            steps.push(taken);
            if !more {
                break;
            }
        }
        (pos, steps)
    }

    #[test]
    fn glide_reaches_target_smoothly() {
        let s = SmoothScroll::default();
        let t = Instant::now();
        s.push(78., 600., t);
        let (pos, steps) = run(&s, t, 16, None);
        assert!((pos - 78.).abs() < 1e-3, "{pos}");
        assert!(!s.is_active());
        // Adımlar küçülerek gider (son adım kalan yarım pikseli de alır);
        // ~150 ms'de oturur.
        assert!(steps.windows(2).all(|w| w[1] <= w[0] + SNAP_PX), "{steps:?}");
        assert!(steps.len() <= 14, "{} kare", steps.len());
        assert!(steps[0] < 40., "ilk adım {}", steps[0]);
    }

    #[test]
    fn uneven_frame_timing_gives_even_steps() {
        // 180 Hz ekranda çizimler 4,4 / 6,7 ms aralıkla gelse de adımlar
        // düzenli küçülsün.
        let s = SmoothScroll::default();
        let mut now = Instant::now();
        // Önce tahmin otursun.
        s.push(10_000., 10_000., now);
        let mut pos = 0.;
        for i in 0..40 {
            now += Duration::from_micros(if i % 2 == 0 { 4_400 } else { 6_700 });
            let mut taken = 0.;
            assert!(s.tick(now, pos, |d| {
                taken = d;
                d
            }));
            pos += taken;
        }
        s.cancel();
        s.push(78., 600., now);
        let mut steps = Vec::new();
        pos = 0.;
        for i in 0..200 {
            now += Duration::from_micros(if i % 2 == 0 { 4_400 } else { 6_700 });
            let mut taken = 0.;
            let more = s.tick(now, pos, |d| {
                taken = d;
                d
            });
            pos += taken;
            steps.push(taken);
            if !more {
                break;
            }
        }
        assert!((pos - 78.).abs() < 1e-3, "{pos}");
        let ratios: Vec<f32> = steps.windows(2).take(10).map(|w| w[1] / w[0]).collect();
        let (lo, hi) = ratios.iter().fold((f32::MAX, f32::MIN), |(a, b), r| (a.min(*r), b.max(*r)));
        assert!(hi - lo < 0.08, "adım oranları {ratios:?}");
    }

    #[test]
    fn same_direction_accumulates_reverse_resets() {
        let s = SmoothScroll::default();
        let t = Instant::now();
        s.push(78., 600., t);
        s.push(78., 600., t);
        assert_eq!(s.remaining.get(), 156.);
        s.push(-78., 600., t);
        assert_eq!(s.remaining.get(), -78.);
        // Biriken yol görünen alanın iki katını aşmaz.
        for _ in 0..40 {
            s.push(-78., 600., t);
        }
        assert_eq!(s.remaining.get(), -1200.);
    }

    #[test]
    fn stops_at_the_edge_and_when_moved_elsewhere() {
        let s = SmoothScroll::default();
        let t = Instant::now();
        s.push(200., 600., t);
        let (pos, _) = run(&s, t, 16, Some(50.));
        assert_eq!(pos, 50.);
        assert!(!s.is_active());

        // Konum dışarıdan değişti (ör. klavye): hareket bırakılır.
        s.push(200., 600., t);
        assert!(s.tick(t + Duration::from_millis(16), 0., |d| d));
        assert!(!s.tick(t + Duration::from_millis(32), 500., |d| d));
        assert!(!s.is_active());
    }
}
