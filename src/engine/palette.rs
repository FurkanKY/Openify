//! Kapak resminden vurgu rengi.
//!
//! Resim 24×24'e küçültülür; yeterince doygun ve ne çok koyu ne çok açık
//! pikseller ton dilimlerine (12 dilim) ayrılır, en ağır dilimin ortalama
//! rengi seçilir. Sonuç iki renge çevrilir:
//!   - `text`: koyu zeminde okunacak kadar açık vurgu (çalan şarkı, çubuk)
//!   - `glow`: başlıktaki ışıma için koyu, doygun ton
//! Kapak gri tonluysa (siyah-beyaz fotoğraf vb.) `None`: varsayılan vurgu kalır.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    /// 0xRRGGBB
    pub text: u32,
    /// 0xRRGGBB
    pub glow: u32,
}

const HUE_BUCKETS: usize = 12;
const MIN_SATURATION: f32 = 0.22;

pub fn accent_from_image(bytes: &[u8]) -> Option<Accent> {
    let image = image::load_from_memory(bytes).ok()?;
    let small = image.thumbnail_exact(24, 24).to_rgb8();
    let pixels: Vec<[u8; 3]> = small.pixels().map(|p| p.0).collect();
    accent_from_pixels(&pixels)
}

fn accent_from_pixels(pixels: &[[u8; 3]]) -> Option<Accent> {
    let mut weight = [0f32; HUE_BUCKETS];
    let mut sum = [[0f32; 3]; HUE_BUCKETS];
    for &[r, g, b] in pixels {
        let (h, s, l) = rgb_to_hsl(r, g, b);
        if s < MIN_SATURATION || !(0.15..=0.85).contains(&l) {
            continue;
        }
        let bucket = ((h / 360.0) * HUE_BUCKETS as f32) as usize % HUE_BUCKETS;
        // Doygun ve orta parlaklıktaki pikseller daha çok sayılır.
        let w = s * (1.0 - (l - 0.5).abs() * 1.4);
        weight[bucket] += w;
        sum[bucket][0] += r as f32 * w;
        sum[bucket][1] += g as f32 * w;
        sum[bucket][2] += b as f32 * w;
    }
    let (best, &w) = weight
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    // Resmin küçük bir kısmıysa (ör. siyah-beyaz kapakta tek renkli yazı) sayma.
    if w < pixels.len() as f32 * 0.03 {
        return None;
    }
    let avg = sum[best].map(|c| (c / w).round().clamp(0.0, 255.0) as u8);
    let (h, s, _) = rgb_to_hsl(avg[0], avg[1], avg[2]);
    Some(Accent {
        text: hsl_to_rgb(h, s.clamp(0.45, 0.85), 0.70),
        glow: hsl_to_rgb(h, s.clamp(0.50, 0.90), 0.42),
    })
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hue_of(rgb: u32) -> f32 {
        rgb_to_hsl((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8).0
    }

    #[test]
    fn picks_dominant_saturated_hue() {
        // Çoğu koyu mor, biraz turuncu, bolca siyah.
        let mut px = vec![[20, 20, 20]; 200];
        px.extend(vec![[110, 40, 200]; 300]);
        px.extend(vec![[240, 140, 80]; 76]);
        let accent = accent_from_pixels(&px).unwrap();
        let h = hue_of(accent.text);
        assert!((250.0..300.0).contains(&h), "mor bekleniyordu, ton {h}");
        // Metin vurgusu koyu zeminde okunacak kadar açık.
        let (_, _, l) = rgb_to_hsl(
            (accent.text >> 16) as u8,
            (accent.text >> 8) as u8,
            accent.text as u8,
        );
        assert!(l > 0.6);
    }

    #[test]
    fn grayscale_cover_gives_no_accent() {
        let px: Vec<[u8; 3]> = (0..576).map(|i| [(i % 255) as u8; 3]).collect();
        assert!(accent_from_pixels(&px).is_none());
    }

    #[test]
    fn hsl_roundtrip_keeps_hue() {
        let rgb = hsl_to_rgb(200.0, 0.6, 0.5);
        assert!((hue_of(rgb) - 200.0).abs() < 2.0);
    }
}
