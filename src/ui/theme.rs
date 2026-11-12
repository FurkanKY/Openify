//! Openify'ın görsel kimliği (Adım 8).
//!
//! Yön: Sade > Atmosferik > Editoryal, ilk denemeden sonra "durgun,
//! sıkışık, kişiliksiz" geri bildirimiyle canlandırıldı:
//!   - Sade taban: nötr siyah zemin, alta sabit çalar, ferah boşluklar.
//!   - Atmosferik: vurgu rengi ve ışıma çalan şarkının kapağından, şarkı
//!     değişince yumuşak geçişle; her yerde kapaklar.
//!   - Kişilik: tek bir karakterli yazı tipi ailesi, kalın büyük başlıklar,
//!     vurgu renginde "listeyi çal" düğmesi, canlı ekolayzer.
//!
//! Yazı tipleri ve simgeler (Lucide) exe'ye gömülü; lisansları
//! `assets/licenses/` altında.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{
    App, AssetSource, BoxShadow, FontFeatures, Hsla, Rgba, SharedString, Svg, point, prelude::*,
    px, rgb, rgba, svg,
};

use crate::engine::palette::Accent;

// ── Renkler ─────────────────────────────────────────────────────────────
//
// Pusula: "gece dinleme odası". Oda karanlık ve nötr; tek ışık kaynağı çalan
// şarkının kapağı (vurgu rengi ve ışıma, `engine/palette.rs`). Yüzeyler
// gölgeyle değil ton katmanlarıyla ayrılır: zeminin üstüne beyazın saydam
// tonları (`VEIL_*`). Gölge yalnızca içeriğin üstünde yüzen öğelerde.

/// Oda: pencerenin zemini.
pub const BG: u32 = 0x0E0E10;
/// Çalar hapı ve yüzen paneller (palet, menü, ipucu).
pub const BAR: u32 = 0x121215;
/// Yüzen panellerin ve ayrı bölgelerin kenarı.
pub const BORDER: u32 = 0x1C1C21;
/// Ön plan: bütün yazılar bu rengin saydam tonlarıdır.
pub const TEXT: u32 = 0xF4F4F5;
/// İlerleme ve ses çubuklarının boş kısmı.
pub const TRACK: u32 = 0x2A2A30;
/// Kapak gelene kadar yerini tutan yüzey.
pub const PLACEHOLDER: u32 = 0x1C1C21;
/// Kapat düğmesinin üzerine gelince (Windows'un kendi kırmızısı).
pub const CLOSE_HOVER: u32 = 0xC42B1C;
/// Tehlikeli işlemlerin yazısı (silme, çıkarma): yüzen panelde ≥ 4,5:1.
pub const DANGER_TEXT: u32 = 0xF87171;

/// Ton katmanları: zeminin üstüne beyazın saydamlığı. Durumlar her yerde
/// aynı katmanı kullanır (raf, liste, palet, menü).
pub const VEIL_HOVER: f32 = 0.06;
pub const VEIL_SELECTED: f32 = 0.10;
pub const VEIL_PRESSED: f32 = 0.14;
/// Ayırıcı çizgiler (sütun başlığının altı, bölge sınırları).
pub const LINE: f32 = 0.06;
/// Giriş kutusu kenarı; üzerine gelince `LINE_HOVER`.
pub const LINE_INPUT: f32 = 0.10;
pub const LINE_HOVER: f32 = 0.18;
/// Işımanın üstündeki giriş kutusu zemini: siyahın saydamlığı.
pub const SHADE_INPUT: f32 = 0.32;
pub const SHADE_INPUT_HOVER: f32 = 0.42;
/// Palet açıkken arkadaki içeriği karartan perde.
pub const SCRIM: f32 = 0.60;

/// Çalan satırın zemini: kapak ışığının tonu (beyaz katman yerine). Çalan
/// satır üzerine gelince, seçiliyken ve basınca da beyaza dönmez, aynı
/// ışığın daha güçlü tonunu alır: hangi durumda olursa olsun kapağın
/// rengini taşır. (Seçili beyaz katman çalan satırın tonunu siliyordu.)
pub const ACCENT_PLAYING: f32 = 0.08;
pub const ACCENT_PLAYING_HOVER: f32 = 0.12;
pub const ACCENT_PLAYING_SELECTED: f32 = 0.16;
pub const ACCENT_PLAYING_PRESSED: f32 = 0.20;
/// Çalar hapının cam zemini, üstten alta (ışık yukarıdan gelir gibi; 7 Kasım
/// kararı, `pill_glass`). Önce düz `BAR` ve renkli kenar çizgisiydi.
pub const PILL_GLASS_TOP: u32 = 0x19191D;
pub const PILL_GLASS_BOTTOM: u32 = 0x0F0F12;
/// Hapın 1 px'lik iç halkası (beyazın saydamlığı).
pub const PILL_RING: f32 = 0.055;
/// Hapın üstündeki ince ışık çizgisi: halkanın üstüne biner, üstte toplam
/// ~%12 beyaz; yuvarlak uçlarda incelerek söner.
pub const PILL_HIGHLIGHT: f32 = 0.07;
/// Hapın kapak ışığının en parlak noktası (göreli parlaklık). Odanın tavanının
/// (0,025) iki katı: hap yüzen bir yüzey ve ışığın düştüğü yerde etiket
/// tonunda yazı yok, en soluğu ikincil (%64; sanatçı, bildirim satırı); o
/// her kapakta ≥ 4,5:1 (test). Etiket tonundaki süreler ışığın söndüğü yerde.
/// 0,025'te ışık camda zor seçiliyordu.
pub const PILL_LIGHT_MAX_LUMINANCE: f32 = 0.05;
/// Göreli parlaklığı bu tavanın hemen üstünde olan gri (0,0513): haptaki
/// vurgu rengi buna göre okunur kılınır.
pub const PILL_LIGHT_REF: u32 = 0x404040;

/// Haptaki vurgu: kapak ışığının en parlak yerinde de ≥ 4,5:1. Işıma
/// koyu bir tondaysa (lacivert) vurgu ona göre açılmıştı; hapın ışığı
/// tavana kadar parladığı için biraz daha açılabilir (ton korunur).
pub fn pill_accent(accent: u32) -> u32 {
    ensure_contrast(accent, PILL_LIGHT_REF, 4.5)
}

/// Kapak ışığının `alpha` saydamlığı (çalan satır).
pub fn accent_veil(accent: u32, alpha: f32) -> Rgba {
    with_alpha(accent, alpha)
}

/// Ton katmanı: beyazın `alpha` saydamlığı.
pub fn veil(alpha: f32) -> Rgba {
    with_alpha(0xFFFFFF, alpha)
}

/// Siyahın `alpha` saydamlığı (ışımanın üstünde koyulaştırma, perde).
pub fn shade(alpha: f32) -> Rgba {
    with_alpha(0x000000, alpha)
}

/// Hapın arkasındaki alt perde: pencerenin alt kenarından yukarı, hap payı
/// (`PILL_CLEARANCE`) + `PILL_SCRIM_FADE` kadar yükselir; üstte saydam, hapın
/// arkasında `PILL_SCRIM_ALPHA` kadar siyah. Hapın altından geçen satırlar
/// hafifçe söner, hap öne çıkar (gölgesi tek başına yetmiyordu: yazılar
/// hapın yanından ve altından kesik görünüyordu).
pub const PILL_SCRIM_ALPHA: f32 = 0.5;
/// Perdenin hap payının üstünde, saydamdan başlayan yumuşak geçiş yüksekliği.
pub const PILL_SCRIM_FADE: f32 = S6;

/// Işımanın yüksekliği (Atmosferik yönün ağırlığı burada).
pub const GLOW_HEIGHT: f32 = 460.;
/// Işımanın gücü: zeminle bu oranda karışır. 0,58'de bütün pencereyi
/// kahverengiye boyayıp okunurluğu düşürüyordu.
pub const GLOW_ALPHA: f32 = 0.36;
/// Işımanın en parlak yerinin göreli parlaklık tavanı. Sarı ya da pembe gibi
/// parlak kapaklar aynı oranda odayı fazla aydınlatıyor ve üstündeki
/// etiketleri 4,5:1'in altına düşürüyordu (sarıda 4,2:1). Oda gece kalır.
pub const GLOW_MAX_LUMINANCE: f32 = 0.025;

/// Işımanın en üstteki (en parlak) rengi: zeminle karışım, parlaklık
/// tavanını aşmayacak kadar.
pub fn glow_top(glow: u32) -> u32 {
    let mut t = GLOW_ALPHA;
    loop {
        let top = mix(BG, glow, t);
        if luminance(top) <= GLOW_MAX_LUMINANCE || t <= 0.0 {
            return top;
        }
        t -= 0.01;
    }
}

/// Kapaktan renk çıkarılamazsa kullanılan vurgu.
pub const DEFAULT_ACCENT: Accent = Accent {
    text: 0xF59E5B,
    glow: 0xC2410C,
};

/// Henüz hiçbir şey çalmamışken: oda karanlık (ışıma zeminin kendisi).
/// Işık ilk şarkının kapağıyla yanar. Yazı vurgusu (Çal düğmesi, odak
/// halkası) varsayılan renkte kalır: denetimler karanlıkta da okunsun.
pub const DARK_ROOM: Accent = Accent {
    text: DEFAULT_ACCENT.text,
    glow: BG,
};

/// 0xRRGGBB + saydamlık (0.0–1.0)
pub fn with_alpha(color: u32, alpha: f32) -> Rgba {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    rgba((color << 8) | a)
}

/// HSL (ton 0–360°, doygunluk ve açıklık 0–1) → 0xRRGGBB. Adan türetilen
/// sakin tonlar için (arkadaşın profil resmi yokken baş harf dairesi).
pub fn hsl_color(hue: f32, saturation: f32, lightness: f32) -> u32 {
    let h = hue.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = lightness - c / 2.0;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

/// `a` ile `b` arasında `t` oranında karışım (0.0 = a, 1.0 = b).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xFF) as f32;
        let y = ((b >> shift) & 0xFF) as f32;
        ((x + (y - x) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// WCAG göreli parlaklık (0.0 siyah – 1.0 beyaz).
pub fn luminance(color: u32) -> f32 {
    let lin = |shift: u32| {
        let c = ((color >> shift) & 0xFF) as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(16) + 0.7152 * lin(8) + 0.0722 * lin(0)
}

/// İki rengin WCAG karşıtlık oranı (1:1 – 21:1).
pub fn contrast(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Rengi `bg` üzerinde en az `min` karşıtlığa ulaşana kadar beyaza doğru
/// açar. Tonu korur; zaten yeterliyse dokunmaz.
pub fn ensure_contrast(color: u32, bg: u32, min: f32) -> u32 {
    if contrast(color, bg) >= min {
        return color;
    }
    for step in 1..=20 {
        let lighter = mix(color, 0xFFFFFF, step as f32 / 20.0);
        if contrast(lighter, bg) >= min {
            return lighter;
        }
    }
    0xFFFFFF
}

// ── Boşluk ölçeği ───────────────────────────────────────────────────────
//
// Bütün boşluklar bu basamaklardan seçilir. Yakın ilişkili öğeler küçük, ayrı
// gruplar büyük basamakla ayrılır. `S0` yalnızca aynı öğenin iki satırı
// arasında (ad ile alt satırı) ve sıkı listelerde öğeler arasında.

pub const S0: f32 = 2.;
pub const S1: f32 = 4.;
pub const S2: f32 = 8.;
pub const S3: f32 = 12.;
pub const S4: f32 = 16.;
pub const S5: f32 = 24.;
pub const S6: f32 = 32.;
pub const S7: f32 = 48.;
pub const S8: f32 = 64.;

/// Sayfa kenar boşluğu (sol ve sağ), içerik alanının eninden: şarkı listesi,
/// raf, sütun başlıkları, sayfa kartları aynı değeri kullanır, kenarlar hizalı
/// kalır. Dar pencerede 32 px (kenar çubuğu açıkken bile içerik 1180'de
/// rahat), 1280 px'ten 48, 1600 px'ten 64: geniş ekranda liste kenardan
/// kenara uzamaz, ölçülü bir çerçeve içinde kalır; kenar boşluğu içeriğin
/// yaklaşık %4'ü civarında tutulur. Geçişler eşikten önceki 64 px'te yavaşça
/// olur (7 Kasım): yerleşik panel kayarak açılırken ya da pencere
/// boyutlandırılırken kenar zıplamaz. `width`: yan panelin yerleşik olduğu
/// durumda panelin kapladığı yer çıkmış içerik eni (`Shell::window_width`).
pub fn page_margin(width: f32) -> f32 {
    const RAMP: f32 = 64.;
    let toward = |threshold: f32, from: f32, to: f32| {
        from + (to - from) * ((width - (threshold - RAMP)) / RAMP).clamp(0., 1.)
    };
    if width >= 1600. {
        S8
    } else if width >= 1600. - RAMP {
        toward(1600., S7, S8).round()
    } else if width >= 1280. {
        S7
    } else if width >= 1280. - RAMP {
        toward(1280., S6, S7).round()
    } else {
        S6
    }
}

// ── Köşe yarıçapları ────────────────────────────────────────────────────

/// İnce çubuklar (ilerleme, ses, ekolayzer).
pub const R_XS: f32 = 2.;
/// Küçük kapaklar, kısayol etiketi.
pub const R_SM: f32 = 4.;
/// Satırlar, düğme zeminleri, menü öğeleri.
pub const R_MD: f32 = 6.;
/// Raf kapakları, "Tümü" karosu, arama kutusu.
pub const R_LG: f32 = 8.;
/// Yüzen paneller (palet, menü).
pub const R_XL: f32 = 12.;

// ── Derinlik ────────────────────────────────────────────────────────────

/// Yüzen öğelerin gölgesi: palet, menü, ipucu, başlık kapağı. Başka hiçbir
/// yüzeyde gölge yok; gölge "bu geçici, içeriğin üstünde" demek.
pub fn float_shadow() -> Vec<BoxShadow> {
    // Tek katman. Önceki ikinci katman (0 2 6 siyah %30) "şeffaf delik"
    // yüzünden kaldırılmıştı; 7 Kasım'da ölçüldü: delikler gpui'nin değil,
    // ekran görüntüsü betiğinin (GDI+ #0D0B0C'yi saydam sayıyor; bkz.
    // vendor/gpui/OPENIFY.md madde 7). İki katman da doğru çiziliyor.
    vec![BoxShadow {
        color: shade(0.55).into(),
        offset: point(px(0.), px(12.)),
        blur_radius: px(32.),
        spread_radius: px(0.),
    }]
}

/// Yerleşik "Şimdi çalıyor" panelinin gölgesi (7 Kasım, akşam): yüzen
/// gölgenin sakin hâli. Yerleşik panel listenin yanında, içeriğin üstünde
/// değil; yüzen gölge (12 / 32 / %55) onu geçici bir katman gibi
/// gösteriyordu. Bu gölge yalnız paneli zeminden bir kat kaldırır: kenarları
/// gece zemininde belli olsun, yüzüyormuş gibi durmasın.
pub fn dock_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: shade(0.40).into(),
        offset: point(px(0.), px(8.)),
        blur_radius: px(24.),
        spread_radius: px(0.),
    }]
}

/// Hap, bildirim şeridi ve kaydırıcı tutamacı gölgesi: tek katman.
///
/// (Hapın üstünde görülen 1 px'lik "şeffaf çizgiler" gölgeden değil, ekran
/// görüntüsü betiğinin GDI+ yolundandı: tam #0D0B0C'ye düşen pikseller
/// (0, 0, 0, 0) görünüyordu, ekranda yoktu; bkz. vendor/gpui/OPENIFY.md
/// madde 7.)
pub fn pill_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: shade(0.55).into(),
        offset: point(px(0.), px(8.)),
        blur_radius: px(28.),
        spread_radius: px(0.),
    }]
}

/// Cam hapın kendi gölgesi (7 Kasım): daha derin ve yumuşak, yine tek katman
/// (`pill_shadow`'un 8 / 28 / %55'inden 12 / 36 / %60'a): hap içeriğin
/// üstünde daha belirgin yüzer. Kaydırıcı tutamacı küçük, eskisinde kalır.
pub fn capsule_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: shade(0.60).into(),
        offset: point(px(0.), px(12.)),
        blur_radius: px(36.),
        spread_radius: px(0.),
    }]
}

// ── Hareket ─────────────────────────────────────────────────────────────
//
// Hareket jetonları (7 Kasım akşam): giriş yavaşlayarak biter (ease-out
// quint / üstel), çıkış girişten kısa ve hızlanarak gider (ease-in), basit
// durum renkleri 120–150 ms. Hiçbiri girdiyi bekletmez; sürerken kare ister,
// biter bitmez istemez. Ortam (vurgu rengi, ışıma) ayrı ve daha yavaş.
// Windows'ta animasyonlar kapalıysa (Erişilebilirlik → Görsel efektler)
// hepsi anında olur.
//
//   `motion_fast`  140 ms  üzerine gelme / basma rengi, sekme geçişi
//   `motion`       220 ms  sayfa, panel girişi, tam ekran girişi, palet
//   `motion_exit`  160 ms  panel / tam ekran çıkışı (ease-in, `cubic_in`)
//   `cover_fade`   150 ms  inen kapağın belirmesi
//   `accent_fade`  800 ms  ortam rengi

/// Üzerine gelme / basma gibi küçük durum renkleri, sekme içeriği geçişi.
pub fn motion_fast() -> std::time::Duration {
    scaled(std::time::Duration::from_millis(140))
}

/// Durum değişimi geçişleri (sayfa, şarkı, palet, panel girişi): kısa,
/// akışı bekletmez.
pub fn motion() -> std::time::Duration {
    scaled(std::time::Duration::from_millis(220))
}

/// Çıkışlar (panel kapanması, tam ekrandan dönüş): girişten kısa, kullanıcı
/// kapattığı şeyin gitmesini beklemez.
pub fn motion_exit() -> std::time::Duration {
    scaled(std::time::Duration::from_millis(160))
}

/// Vurgu rengi bir kapaktan diğerine bu sürede geçer.
pub fn accent_fade() -> std::time::Duration {
    scaled(std::time::Duration::from_millis(800))
}

/// İnen bir kapak yer tutucunun üstünde bu sürede belirir.
pub fn cover_fade() -> std::time::Duration {
    scaled(std::time::Duration::from_millis(150))
}

fn scaled(d: std::time::Duration) -> std::time::Duration {
    // Kapalıysa 1 ms: geçiş anında biter, süreye bölen hesaplar sıfıra bölmez.
    if animations_enabled() { d.mul_f32(slow_motion()) } else { std::time::Duration::from_millis(1) }
}

/// Geliştirme kancası `OPENIFY_TEST_SLOW_MOTION=<kat>`: bütün geçişler bu
/// kat uzun sürer (geçişin ortasını ekran görüntüsüyle yakalamak için).
fn slow_motion() -> f32 {
    static FACTOR: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *FACTOR.get_or_init(|| {
        std::env::var("OPENIFY_TEST_SLOW_MOTION")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|f| *f >= 1.)
            .unwrap_or(1.)
    })
}

/// Kullanıcının "Animasyonları azalt" tercihi (Ayarlar): 0 = Windows'un
/// ayarını izle, 1 = azalt (animasyon yok), 2 = azaltma (animasyon var).
static MOTION_PREF: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Ayarlardan gelen tercih: `"system"`, `"on"` (azalt), `"off"` (azaltma).
pub fn set_reduce_motion(pref: &str) {
    let v = match pref {
        "on" => 1,
        "off" => 2,
        _ => 0,
    };
    MOTION_PREF.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// Animasyonlar açık mı? Tercih "sistem"se Windows'un "animasyon efektleri"
/// ayarı (açılışta bir kez okunur), değilse kullanıcının seçimi.
pub fn animations_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    animation_choice(MOTION_PREF.load(std::sync::atomic::Ordering::Relaxed), || {
        *ENABLED.get_or_init(read_animation_setting)
    })
}

/// Tercih ve sistem ayarından animasyon kararı (saf; test edilir).
fn animation_choice(pref: u8, system: impl FnOnce() -> bool) -> bool {
    match pref {
        1 => false,
        2 => true,
        _ => system(),
    }
}

#[cfg(windows)]
fn read_animation_setting() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut enabled = windows::core::BOOL::from(true);
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || enabled.as_bool()
}

#[cfg(not(windows))]
fn read_animation_setting() -> bool {
    true
}

/// Başı hızlı, sonu yavaş yumuşama (0.0–1.0).
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Giriş eğrisi: çok hızlı başlar, uzun yavaşlar (ease-out quint). Panel,
/// sayfa ve tam ekran girişlerinde; `ease_out` (kübik) küçük renk geçişleri
/// için kalır.
pub fn quint_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}

/// Çıkış eğrisi: yavaş kalkar, hızlanarak gider (ease-in kübik).
pub fn cubic_in(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

// ── Yazı ölçeği ─────────────────────────────────────────────────────────
//
// Altı basamak (Adım 9 kararı). Kodda bunların dışında yazı boyutu yok.

#[derive(Clone, Copy)]
pub struct TypeRole {
    pub size: f32,
    pub line: f32,
}

/// Etiket: sütun başlığı, bölüm adı, süre, kısayol.
pub const T_LABEL: TypeRole = TypeRole { size: 12., line: 16. };
/// İkincil: sanatçı, meta satırı, açıklama.
pub const T_SECONDARY: TypeRole = TypeRole { size: 13., line: 18. };
/// Ana metin: şarkı adı, liste adı, menü öğesi.
pub const T_BODY: TypeRole = TypeRole { size: 14., line: 20. };
/// Palet girişi, kart başlığı.
pub const T_TITLE: TypeRole = TypeRole { size: 18., line: 24. };
/// Dar pencerede sayfa başlığı, bölüm başlığı (Figtree, kalın).
pub const T_HEADLINE: TypeRole = TypeRole { size: 28., line: 34. };
/// Sayfa başlığı (Figtree, kalın).
pub const T_DISPLAY: TypeRole = TypeRole { size: 40., line: 44. };

/// Yazı rolleri ve küçük yazı ayarları için kısayollar.
pub trait Type: Styled {
    /// Yazı boyutu ve satır yüksekliği bu rolden.
    fn type_role(self, role: TypeRole) -> Self {
        self.text_size(px(role.size)).line_height(px(role.line))
    }

    /// Eşit genişlikte rakamlar: süre ve sayılar değişirken titremesin.
    fn tabular(mut self) -> Self {
        self.text_style().get_or_insert_with(Default::default).font_features =
            Some(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));
        self
    }
}

impl<T: Styled> Type for T {}

// ── Yazı tipleri ────────────────────────────────────────────────────────

/// Yazı tipi ailesi. Varsayılan Figtree; Plus Jakarta Sans da gömülü ve
/// `OPENIFY_FONT=jakarta` ile seçilebiliyor (ikisi arasında karar verilmedi,
/// ikisi de kenarda duruyor).
pub fn font() -> &'static str {
    static FONT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FONT.get_or_init(|| match std::env::var("OPENIFY_FONT").as_deref() {
        Ok("jakarta") => "Plus Jakarta Sans",
        _ => "Figtree",
    })
}

pub fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../../assets/fonts/PlusJakartaSans-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/PlusJakartaSans-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/PlusJakartaSans-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/PlusJakartaSans-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/PlusJakartaSans-ExtraBold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Figtree-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Figtree-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Figtree-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Figtree-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Figtree-ExtraBold.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        log::warn!("Yazı tipleri yüklenemedi, sistem yazı tipi kullanılacak: {e:#}");
    }
}

// ── Simgeler ────────────────────────────────────────────────────────────

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "play", "pause", "skip-back", "skip-forward", "volume-2", "volume-1", "volume-x", "search",
    "music", "heart", "heart-fill", "x", "minus", "square", "copy", "disc-3", "list-music",
    "wifi-off", "circle-alert", "info", "refresh-cw",
    // Çalma kontrolleri için dolu simgeler (Lucide'de yok, aynı ölçüde çizildi).
    "play-fill", "pause-fill", "skip-back-fill", "skip-forward-fill",
    // Şimdi çalıyor (Adım 11 / 14): karıştırma, tekrar, sıra, sözler, tam ekran.
    "shuffle", "repeat", "repeat-1", "mic-vocal", "maximize-2", "minimize-2", "grip-vertical",
    // Gezinme ve kütüphane yönetimi (Adım 12 / 13): geri / ileri, ana sayfa,
    // sanatçı, sağ tık menüsü, eşitleme.
    "arrow-left", "arrow-right", "chevron-right", "house", "user", "list-plus", "list-end",
    "list-start", "link", "pencil", "trash-2", "ellipsis", "check", "cloud-upload", "plus",
    // Gezinme sekmeleri, Kütüphane görünümü (Adım 12, 7 Kasım).
    "compass", "library", "layout-grid", "list", "arrow-up-down",
    // Ayarlar sayfası ve profil menüsü (Adım 15).
    "settings", "chevron-down", "log-out", "folder-open",
    // Sanatçı takibi ve paylaşma: takip et / takip ediliyor, Spotify'da aç.
    "user-plus", "user-check", "external-link",
    // Karıştırma kuralları: "Bu sanatçıyı hiç çalma".
    "user-x",
    // Karşılama ekranı: uygulama işareti (karosuz).
    "openify-mark",
    // Arkadaş etkinliği (başlık çubuğu düğmesi).
    "users",
    // Kütüphane düzeni: klasör, sabitleme.
    "folder", "pin", "pin-off",
    // Kütüphane araçları: araç çubuğu düğmesi ve palet komutu.
    "wrench",
    // Radyo, gizleme, gizli oturum: menü girdileri, başlık çubuğu simgesi.
    "radio", "eye", "eye-off",
    // Bir anlam, bir simge: "Hiçbir yerde çalma" (ban), Connect şeridi (cast).
    "ban", "cast",
);

/// gpui'nin `svg().path(...)` için okuduğu gömülü dosyalar.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

/// Lucide simgesi; rengi `color` ile boyanır.
pub fn icon(name: &'static str, size: f32, color: impl Into<Hsla>) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(color.into())
}

pub fn text() -> Rgba {
    rgb(TEXT)
}

/// İkincil metin: gri değil, ön plan renginin saydam tonu. Kapak renginde
/// ışımanın üstünde de okunur kalır (gri, kahverengi zeminde bulanıklaşıyordu).
pub fn muted() -> Rgba {
    with_alpha(TEXT, 0.64)
}

/// Tam metinden bir ton geri: simge düğmeleri ve başlık çubuğundaki ad.
/// Üzerine gelince tam metne açılır.
pub fn soft() -> Rgba {
    with_alpha(TEXT, 0.86)
}

/// Küçük etiketler (sütun başlıkları, bölüm adları): zeminde ~5,2:1, kapak
/// ışımasının üstünde her renkte ≥ 4,5:1 (bkz. testler).
pub fn label() -> Rgba {
    with_alpha(TEXT, LABEL_ALPHA)
}

pub const LABEL_ALPHA: f32 = 0.52;

/// Pasif simgeler (metinde kullanılmaz).
pub fn disabled() -> Rgba {
    with_alpha(TEXT, 0.28)
}

/// Arayüz metni: Windows'un kendi arayüz yazı tipi. Küçük boyutlar için
/// ipucu verisiyle (hinting) çiziliyor; Figtree 12–14 px'te bulanıktı.
/// Bulunamazsa gpui sistem yazı tipine düşer.
pub const UI_FONT: &str = "Segoe UI Variable Text";

#[cfg(test)]
mod tests {
    use super::*;

    /// Çıta: kapaktan gelen vurgu her durumda ≥ 4,5:1. Bütün tonları ve
    /// açıklıkları tarayan 100 renkle (10 ton × 10 açıklık), ışıma da aynı
    /// renkten geldiğinde (en kötü durum: zemin açılmış).
    #[test]
    fn any_accent_becomes_readable() {
        for hue in 0..10 {
            for light in 0..10 {
                let color = hsl_to_rgb(hue as f32 * 36.0, 0.8, 0.05 + light as f32 * 0.09);
                let glow_top = mix(BG, color, 0.36);
                let fixed = ensure_contrast(color, glow_top, 4.5);
                assert!(
                    contrast(fixed, glow_top) >= 4.5 && contrast(fixed, BG) >= 4.5,
                    "{color:06X} → {fixed:06X}"
                );
            }
        }
    }

    fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match h as u32 / 60 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
        (to(r) << 16) | (to(g) << 8) | to(b)
    }

    /// Çıta: etiketler (en soluk yazı) kapak ışımasının üstünde de ≥ 4,5:1.
    /// Motorun ışıma renkleri (açıklık 0,42, doygunluk 0,5–0,9) bütün tonlarda;
    /// etiketlerin bulunabildiği yükseklikler (başlık çubuğunun ve küçülmüş
    /// başlığın altı, 112 px'ten ışımanın bittiği yere).
    #[test]
    fn labels_stay_readable_on_the_glow() {
        let mut worst = (f32::MAX, 0u32, 0f32);
        for hue in (0..360).step_by(5) {
            for sat in [0.5, 0.7, 0.9] {
                let glow = hsl_to_rgb(hue as f32, sat, 0.42);
                let top = glow_top(glow);
                let mut y = 112.0;
                while y < GLOW_HEIGHT {
                    let bg = mix(top, BG, y / GLOW_HEIGHT);
                    let fg = mix(bg, TEXT, LABEL_ALPHA);
                    let c = contrast(fg, bg);
                    if c < worst.0 {
                        worst = (c, glow, y);
                    }
                    y += 2.0;
                }
            }
        }
        assert!(worst.0 >= 4.5, "{:.2}:1 ışıma {:06X}, y = {}", worst.0, worst.1, worst.2);
    }

    #[test]
    fn every_icon_is_embedded_and_is_svg() {
        for (path, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.contains("<svg"), "{path} SVG değil");
            assert!(!text.contains("<script"), "{path} script içeriyor");
        }
        assert!(Assets.load("icons/play.svg").unwrap().is_some());
        assert!(Assets.load("icons/yok.svg").unwrap().is_none());
    }

    #[test]
    fn reduce_motion_choice_overrides_the_system_setting() {
        // Sistem: Windows ne diyorsa o. Açık: animasyon yok, kapalı: var.
        assert!(animation_choice(0, || true));
        assert!(!animation_choice(0, || false));
        assert!(!animation_choice(1, || true));
        assert!(animation_choice(2, || false));
    }

    #[test]
    fn alpha_is_packed_into_rgba() {
        let c = with_alpha(0xFF0000, 0.5);
        assert!((c.r - 1.0).abs() < 1e-6 && (c.a - 128.0 / 255.0).abs() < 1e-3);
    }

}
