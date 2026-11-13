//! Hapın cam yüzeyi (7 Kasım kararı: "Apple tarzı cam").
//!
//! Temaya göre koyu ya da sıcak fildişi cam zemin (üstten alta koyulaşır),
//! üstte ince ışık çizgisi, 1 px'lik soluk iç halka ve derin yumuşak gölge
//! (`theme::pill_shadow`, tek katman). Renkli kenar çizgisi yok.
//!
//! Bir şey çalarken kapağın durduğu soldan kapağın kendi renkleriyle yumuşak
//! bir ışık yayılır (Apple Music'in mini çalarındaki gibi): kapak 32 px'e
//! küçültülüp bulanıklaştırılır, her noktanın tonu korunur (doygunluğu
//! kapaktaki renk gücünden), parlaklığı odanın tavanına çekilir; ışık
//! kapağın ortasından elips biçiminde yumuşakça söner. Gri ya da siyah
//! kapakta ışık nötr kalır, koyu kapakta zemin koyulaşmaz (ışık yoksa
//! sakin cam).
//!
//! gpui'de bulanıklık ve dairesel degrade yok: yüzey ayrı bir iş
//! parçacığında hapın en geniş hâlinin boyunda (800 × 72) bir kez hazırlanır
//! ve doğrudan `RenderImage` olarak hapın altına yuvarlak resim olarak çizilir
//! (dar hapta yatayda biraz sıkışır). Şarkı başına bir kez; kare başına iş
//! yok. Şarkı değişince yeni ışık eskisinin üstünde belirir (ortam geçişi,
//! vurgu rengiyle aynı 0,8 sn; "animasyonları azalt"ta anında); eskisi geçiş
//! bitince gpui'nin doku atlasından da atılır (`App::drop_image`).
//!
//! Açık temada düşük yoğunluklu renk lekeleri camla karışır ve karanlık
//! yazının okunması için parlaklık tabanı korunur. Koyu temada ışığın en
//! parlak noktası göreli parlaklık 0,05'i geçmez
//! (`PILL_LIGHT_MAX_LUMINANCE`); ışığın düştüğü yerdeki en soluk yazı
//! (ikincil, %64) her kapakta ≥ 4,5:1, etiket tonundaki süreler ışığın
//! söndüğü yerde ≥ 4,5:1, haptaki vurgu rengi de ışığa göre ≥ 4,5:1
//! (`pill_accent`; test).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{AnyElement, Context, ObjectFit, RenderImage, Window, div, img, prelude::*};
use image::imageops::FilterType;

use super::{
    Shell,
    fullscreen::box_blur,
    pill::{PILL_COVER, PILL_HEIGHT, PILL_MAX_WIDTH},
    theme::*,
};

/// Işık resminin boyu: hapın en geniş hâli (mantıksal px).
const W: usize = PILL_MAX_WIDTH as usize;
const H: usize = PILL_HEIGHT as usize;
/// Işığın merkezi: kapağın ortası (hapın sol iç boşluğu + kapağın yarısı).
const LIGHT_X: f32 = S3 + PILL_COVER / 2.;
/// Işığın yatay yarıçapı: hapın en geniş hâlinin %60'ında söner (eski
/// soldan sağa degradenin bittiği yer). Dikeyde hap boyunun 1,35 katı: kapağın
/// üstü ve altı da aydınlık (%68), ışık kapağın çevresinde toplanır.
const LIGHT_RX: f32 = PILL_MAX_WIDTH * 0.6;
const LIGHT_RY: f32 = PILL_HEIGHT * 1.35;
/// Kapaktan gelen rengin camla en güçlü karışımı.
const LIGHT_MAX_MIX: f32 = 0.6;
/// Işık renginin açıklığı (motorun ışıma rengindeki gibi orta).
const LIGHT_LIGHTNESS: f32 = 0.45;
/// Kapak bu boya küçültülüp bulanıklaştırılır (renk örnekleri).
const GRID: usize = 32;
/// Hesapta kullanılan tavan: titreşim ve yuvarlama en çok 1 basamak ekler
/// (~0,0011), çizilen piksel `PILL_LIGHT_MAX_LUMINANCE`'ı geçmesin.
const CAP: f32 = PILL_LIGHT_MAX_LUMINANCE - 0.0011;

/// Bir şarkının ışığı ve geldiği an (geçiş için).
pub(super) struct Light {
    pub image: Arc<RenderImage>,
    pub at: Instant,
}

/// Çalan şarkının ışığı; yenisi belirirken bir öncekisi altta durur.
#[derive(Default)]
pub(super) struct Lights {
    pub current: Option<Light>,
    pub previous: Option<Light>,
    seq: u64,
}

impl Shell {
    /// Tema değişince eski temanın iki geçiş katmanını da atlasından bırak.
    pub(super) fn clear_pill_lights(&mut self, cx: &mut Context<Self>) {
        let lights = &mut self.now.full.pill_light;
        lights.seq = lights.seq.wrapping_add(1);
        for old in [lights.current.take(), lights.previous.take()]
            .into_iter()
            .flatten()
        {
            cx.drop_image(old.image, None);
        }
        self.notify_pill(cx);
    }

    /// Yeni şarkının ışığı hazır (bkz. `compute_backdrop`).
    pub(super) fn set_pill_light(&mut self, image: Arc<RenderImage>, cx: &mut Context<Self>) {
        let lights = &mut self.now.full.pill_light;
        if let Some(old) = lights.previous.take() {
            cx.drop_image(old.image, None);
        }
        lights.seq += 1;
        let seq = lights.seq;
        lights.previous = lights.current.replace(Light {
            image,
            at: Instant::now(),
        });
        // Geçiş bitince eskisi bellekten (doku atlasından) atılır.
        if lights.previous.is_some() {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(accent_fade() + Duration::from_millis(100))
                    .await;
                let _ = this.update(cx, |shell, cx| {
                    let lights = &mut shell.now.full.pill_light;
                    if lights.seq == seq
                        && let Some(old) = lights.previous.take()
                    {
                        cx.drop_image(old.image, None);
                        shell.notify_pill(cx);
                    }
                });
            })
            .detach();
        }
        self.notify_pill(cx);
    }

    /// Hapın yüzey katmanları: içerikten önce (altta) çizilir. Zemin (cam
    /// degradesi) hapın kendisinde; burada kapağın ışığı (çalarken), iç halka
    /// ve üst ışık çizgisi. Işık geçişi sürüyorsa hap bir kare daha çizilir.
    pub(super) fn pill_surface(&self, lit: bool, window: &Window) -> Vec<AnyElement> {
        let layer = || div().absolute().top_0().left_0().size_full().rounded_full();
        let picture = |image: &Arc<RenderImage>, opacity: f32| {
            img(image.clone())
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded_full()
                .object_fit(ObjectFit::Fill)
                .opacity(opacity)
                .into_any_element()
        };
        let mut layers = Vec::new();
        let lights = &self.now.full.pill_light;
        if lit && let Some(current) = &lights.current {
            let t = ease_in_out(current.at.elapsed().as_secs_f32() / accent_fade().as_secs_f32());
            if t < 1. {
                window.request_animation_frame();
                if let Some(previous) = &lights.previous {
                    layers.push(picture(&previous.image, 1.));
                }
            }
            layers.push(picture(&current.image, t));
        }
        // İç halka her yanda, ışık çizgisi yalnız üstte (yuvarlak uçlarda
        // incelerek söner: gpui kenarı köşede kalınlıklar arasında geçirir).
        layers.push(
            layer()
                .border_1()
                .border_color(veil(PILL_RING))
                .into_any_element(),
        );
        layers.push(
            layer()
                .border_t_1()
                .border_color(veil(PILL_HIGHLIGHT))
                .into_any_element(),
        );
        layers
    }
}

/// Kapak baytlarından (JPEG / PNG) hapın ışıklı yüzeyi, gpui'nin beklediği
/// BGRA düzeninde. Engellemeli: arka planda çağır.
pub(super) fn light_image(bytes: &[u8], colors: Palette) -> Option<Arc<RenderImage>> {
    let pixels = light_pixels(bytes, colors)?;
    let mut out = image::RgbaImage::new(W as u32, H as u32);
    for (dst, src) in out.pixels_mut().zip(pixels) {
        *dst = image::Rgba([src[2], src[1], src[0], 255]);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(out)])))
}

/// Yüzeyin pikselleri (RGB, satır satır, `W × H`).
fn light_pixels(bytes: &[u8], colors: Palette) -> Option<Vec<[u8; 3]>> {
    let grid = light_grid(bytes, colors)?;
    let mut out = Vec::with_capacity(W * H);
    for y in 0..H {
        let ty = (y as f32 + 0.5) / H as f32;
        // Cam zemini (hapın kendi degradesiyle aynı), kesirli: bantlaşmasın.
        let (top, bottom) = (
            unpack(colors.pill_glass_top),
            unpack(colors.pill_glass_bottom),
        );
        let base = [0, 1, 2].map(|k| top[k] + (bottom[k] - top[k]) * ty);
        for x in 0..W {
            let dx = (x as f32 + 0.5 - LIGHT_X) / LIGHT_RX;
            let dy = (y as f32 + 0.5 - H as f32 / 2.) / LIGHT_RY;
            let d = (dx * dx + dy * dy).sqrt().min(1.);
            let strength = 1. - d * d * (3. - 2. * d);
            let mut c = base;
            if strength > 0. {
                let u = ((x as f32 + 0.5) / LIGHT_RX).min(1.) * (GRID - 1) as f32;
                let v = ty * (GRID - 1) as f32;
                let (color, amount) = sample(&grid, u, v);
                let t = amount * strength;
                c = [0, 1, 2].map(|k| base[k] + (color[k] - base[k]) * t);
                // Örnekler arası karışım tavanı aşarsa ışık o noktada kısılır.
                let mut s = 1.;
                while (if colors.light {
                    lum(c) < COVER_TINT_MIN_LUMINANCE
                } else {
                    lum(c) > CAP
                }) && s > 0.
                {
                    s -= 0.05;
                    c = [0, 1, 2].map(|k| base[k] + (color[k] - base[k]) * t * s);
                }
            }
            let n = dither(y * W + x);
            let mut pixel = c.map(|v| (v + n).round().clamp(0., 255.) as u8);
            // Açık zemindeki taban son 8 bit piksel için de geçerli.
            // Titreşim + yuvarlama, sınırdaki laciverti ~0,005 düşürebilir.
            while colors.light && lum(pixel.map(f32::from)) < COVER_TINT_MIN_LUMINANCE {
                pixel = pixel.map(|v| v.saturating_add(1));
            }
            out.push(pixel);
        }
    }
    Some(out)
}

/// Kapağın bulanık renk ızgarası: her örnek için ışık rengi ve camla en
/// güçlü karışımı (tavanı aşmadan).
fn light_grid(bytes: &[u8], colors: Palette) -> Option<Vec<([f32; 3], f32)>> {
    let source = image::load_from_memory(bytes).ok()?.to_rgb8();
    let small = image::imageops::resize(&source, GRID as u32, GRID as u32, FilterType::Triangle);
    let mut buf: Vec<[f32; 3]> = small
        .pixels()
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
        .collect();
    // Güçlü bulanıklık: kapak hapta yatayda ~15 kat, dikeyde ~2 kat
    // gerilir; ince ayrıntı (ör. ortadaki kırmızı bir tekne) yatay bir
    // şerit olarak görünüyordu. Geriye kapağın birkaç geniş renk lekesi kalır.
    for _ in 0..3 {
        box_blur(&mut buf, GRID, 6);
    }
    let glass = unpack(colors.pill_glass_top);
    let glass_lum = lum(glass);
    Some(
        buf.iter()
            .map(|c| {
                let (h, _, l) = rgb_to_hsl(*c);
                let max = c[0].max(c[1]).max(c[2]) / 255.;
                let min = c[0].min(c[1]).min(c[2]) / 255.;
                let chroma = max - min;
                // Renk gücü: doygunluk kapaktaki renkten (koyu bir pikselin
                // gürültü tonu doymuş sayılmasın diye HSL doygunluğu değil
                // renk farkı). Renksiz örnek kendi açıklığında kalır: siyah
                // kapak ışık vermez, beyaz kapak nötr ışık verir.
                // Bulanıklık renkleri karıştırıp soldurur: Apple'ın mini
                // çalarındaki gibi doygunluk belirgin artırılır.
                let w = (chroma * 5.).min(1.);
                if colors.light {
                    // Fildişinin üstünde düşük yoğunluklu kapak lekeleri.
                    // Renksiz kapak sıcak camı kirletmez.
                    return (*c, PILL_LIGHT_TINT_MIX * w);
                }
                let saturation = (chroma * 3.2).min(0.9);
                let lightness = LIGHT_LIGHTNESS * w + l.min(LIGHT_LIGHTNESS) * (1. - w);
                let color = hsl_to_rgb(h, saturation, lightness);
                if lum(color) <= glass_lum {
                    return (color, 0.);
                }
                let mut amount = LIGHT_MAX_MIX;
                while amount > 0.
                    && lum([0, 1, 2].map(|k| glass[k] + (color[k] - glass[k]) * amount)) > CAP
                {
                    amount -= 0.01;
                }
                (color, amount.max(0.))
            })
            .collect(),
    )
}

/// Izgaradan çift doğrusal örnek.
fn sample(grid: &[([f32; 3], f32)], u: f32, v: f32) -> ([f32; 3], f32) {
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(GRID - 1), (y0 + 1).min(GRID - 1));
    let (tx, ty) = (u.fract(), v.fract());
    let at = |x: usize, y: usize| grid[y * GRID + x];
    let lerp = |a: ([f32; 3], f32), b: ([f32; 3], f32), t: f32| {
        (
            [0, 1, 2].map(|k| a.0[k] + (b.0[k] - a.0[k]) * t),
            a.1 + (b.1 - a.1) * t,
        )
    };
    lerp(
        lerp(at(x0, y0), at(x1, y0), tx),
        lerp(at(x0, y1), at(x1, y1), tx),
        ty,
    )
}

fn unpack(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 0xFF) as f32,
        ((c >> 8) & 0xFF) as f32,
        (c & 0xFF) as f32,
    ]
}

/// Kesirli kanallarla (0–255) WCAG göreli parlaklık.
fn lum(c: [f32; 3]) -> f32 {
    let lin = |v: f32| {
        let v = (v / 255.).clamp(0., 1.);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

fn rgb_to_hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0] / 255., c[1] / 255., c[2] / 255.);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.;
    let d = max - min;
    if d < 1e-6 {
        return (0., 0., l);
    }
    let s = if l > 0.5 {
        d / (2. - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6. } else { 0. }
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    (h * 60., s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    [r, g, b].map(|v| ((v + m) * 255.).clamp(0., 255.))
}

/// Karanlık, geniş degradede bantlaşma olmasın: yarım basamaklık titreşim.
fn dither(i: usize) -> f32 {
    let mut x = i as u32 ^ 0x9E37_79B9;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x & 0xFF) as f32 / 255. - 0.5
}

#[cfg(test)]
mod tests {
    use image::ImageEncoder;

    use super::*;

    fn light_pixels(bytes: &[u8]) -> Option<Vec<[u8; 3]>> {
        super::light_pixels(bytes, Palette::DARK)
    }

    fn jpeg(width: u32, height: u32, color: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |x, y| image::Rgb(color(x, y)));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut out)
            .write_image(&img, width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    fn pack(p: [u8; 3]) -> u32 {
        ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32
    }

    /// Koyu, parlak, doygun, renksiz ve çok renkli kapaklar.
    fn covers() -> Vec<(&'static str, Vec<u8>)> {
        let solid = |c: [u8; 3]| jpeg(64, 64, move |_, _| c);
        vec![
            ("beyaz", solid([255, 255, 255])),
            ("sarı", solid([250, 220, 40])),
            ("pembe", solid([240, 60, 160])),
            ("kırmızı", solid([230, 20, 20])),
            ("mavi", solid([30, 60, 255])),
            ("yeşil", solid([20, 230, 60])),
            ("lacivert", solid([16, 16, 64])),
            ("siyah", solid([6, 6, 8])),
            ("gri", solid([128, 128, 128])),
            (
                "dört renk",
                jpeg(64, 64, |x, y| match (x < 32, y < 32) {
                    (true, true) => [250, 220, 40],
                    (false, true) => [30, 60, 255],
                    (true, false) => [240, 60, 160],
                    (false, false) => [20, 230, 60],
                }),
            ),
        ]
    }

    /// Çıta: hangi kapak olursa olsun hap koyu kalır; ışığın düştüğü yerde
    /// en soluk yazı (ikincil, %64) her noktada ≥ 4,5:1; etiket tonundaki
    /// süreler (ilerlemenin solu, hapın sol ucundan ≥ 470 px) ≥ 4,5:1.
    #[test]
    fn pill_light_keeps_text_readable() {
        const TIMES_FROM: usize = 470;
        for (name, bytes) in covers() {
            let pixels = light_pixels(&bytes).expect("ışık");
            assert_eq!(pixels.len(), W * H);
            let (mut secondary, mut labels) = (f32::MAX, f32::MAX);
            for (i, p) in pixels.iter().enumerate() {
                let c = pack(*p);
                assert!(
                    luminance(c) <= PILL_LIGHT_MAX_LUMINANCE,
                    "{name}: {c:06X} {:.4}",
                    luminance(c)
                );
                secondary = secondary.min(contrast(
                    mix(c, Palette::DARK.text, Palette::DARK.muted_alpha),
                    c,
                ));
                if i % W >= TIMES_FROM {
                    labels = labels.min(contrast(
                        mix(c, Palette::DARK.text, Palette::DARK.label_alpha),
                        c,
                    ));
                }
            }
            assert!(secondary >= 4.5, "{name}: ikincil {secondary:.2}:1");
            assert!(labels >= 4.5, "{name}: etiket {labels:.2}:1");
        }
    }

    /// Eski koyu tavanı açık temaya uygulamak fildişini yok eder ve koyu
    /// yazıyı okunamaz kılar. Renkli ve renksiz kapaklarda gerçek pikselleri sına.
    #[test]
    fn ivory_pill_keeps_quiet_text_and_cover_accents_readable() {
        let colors = Palette::LIGHT;
        for (name, bytes) in covers() {
            let pixels = super::light_pixels(&bytes, colors).expect("surface");
            let mut darkest = colors.pill_glass_top;
            for pixel in pixels {
                let bg = pack(pixel);
                assert!(
                    luminance(bg) >= COVER_TINT_MIN_LUMINANCE,
                    "{name}: {bg:06X}"
                );
                if luminance(bg) < luminance(darkest) {
                    darkest = bg;
                }
                for alpha in [colors.muted_alpha, colors.label_alpha] {
                    let fg = mix(bg, colors.text, alpha);
                    assert!(contrast(fg, bg) >= 4.5, "{name}: {fg:06X}/{bg:06X}");
                }
            }
            for hue in 0..36 {
                let cue = hsl_to_rgb(hue as f32 * 10., 0.8, 0.45).map(|v| v.round() as u8);
                let accent = pill_accent_for(colors, pack(cue));
                assert!(
                    contrast(accent, darkest) >= 4.5,
                    "{name}: {accent:06X}/{darkest:06X}"
                );
            }
        }
    }

    #[test]
    fn ivory_pill_keeps_a_soft_cover_tint() {
        let colors = Palette::LIGHT;
        let pixels = super::light_pixels(&jpeg(64, 64, |_, _| [30, 60, 255]), colors).unwrap();
        let near = pixels[(H / 2) * W + LIGHT_X as usize + 40];
        let far = pixels[(H / 2) * W + W - 40];
        assert!(near[2] as i32 - near[0] as i32 >= 8, "{near:?}");
        assert!(near[0] < far[0], "{near:?}/{far:?}");
    }

    /// Işık görünür (doygun kapakta kapağın yanında rengini taşır), sağ
    /// uçta cama erir; siyah kapakta ışık yok (sakin cam).
    #[test]
    fn pill_light_comes_from_the_cover_and_fades() {
        let at = |pixels: &[[u8; 3]], x: usize, y: usize| pixels[y * W + x];
        let blue = light_pixels(&jpeg(64, 64, |_, _| [30, 60, 255])).unwrap();
        let near = at(&blue, LIGHT_X as usize + 40, H / 2);
        assert!(
            near[2] as i32 - near[0] as i32 >= 40,
            "mavi ışık yok: {near:?}"
        );
        let glass = unpack(mix(
            Palette::DARK.pill_glass_top,
            Palette::DARK.pill_glass_bottom,
            0.5 + 0.5 / H as f32,
        ));
        for pixels in [
            &blue,
            &light_pixels(&jpeg(64, 64, |_, _| [6, 6, 8])).unwrap(),
        ] {
            let far = at(pixels, W - 40, H / 2);
            for k in 0..3 {
                assert!(
                    (far[k] as f32 - glass[k]).abs() <= 1.5,
                    "sağ uç camda değil: {far:?}"
                );
            }
        }
        let black = light_pixels(&jpeg(64, 64, |_, _| [6, 6, 8])).unwrap();
        let center = at(&black, LIGHT_X as usize + 40, H / 2);
        for k in 0..3 {
            assert!(
                (center[k] as f32 - glass[k]).abs() <= 1.5,
                "siyah kapak ışık verdi: {center:?}"
            );
        }
    }

    /// Haptaki vurgu (karıştır / tekrar açıkken simge, odak halkası) ışığın
    /// en parlak yerinde de ≥ 4,5:1.
    #[test]
    fn pill_accent_stays_readable_on_the_light() {
        assert!(luminance(PILL_LIGHT_REF) >= PILL_LIGHT_MAX_LUMINANCE);
        for hue in 0..36 {
            for light in 0..10 {
                let h = hue as f32 * 10.;
                let c = hsl_to_rgb(h, 0.8, 0.05 + light as f32 * 0.09).map(|v| v.round() as u8);
                let fixed = pill_accent_for(Palette::DARK, pack(c));
                assert!(
                    contrast(fixed, PILL_LIGHT_REF) >= 4.5,
                    "{c:?} → {fixed:06X}"
                );
            }
        }
    }
}
