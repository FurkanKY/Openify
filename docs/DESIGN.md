---
name: Openify
description: Spotify hesabını kullanan hızlı, sade, native masaüstü çalar.
colors:
  night-room: "#0E0E10"
  bar-surface: "#121215"
  quiet-edge: "#1C1C21"
  foreground: "#F4F4F5"
  foreground-soft: "rgba(244, 244, 245, 0.86)"
  foreground-muted: "rgba(244, 244, 245, 0.72)"
  foreground-label: "rgba(244, 244, 245, 0.68)"
  foreground-disabled: "rgba(244, 244, 245, 0.28)"
  track-groove: "#2A2A30"
  cover-glow-default: "#C2410C"
  cover-accent-default: "#F59E5B"
  danger-text: "#F87171"
  window-close-red: "#C42B1C"
  veil-hover: "rgba(255, 255, 255, 0.06)"
  veil-selected: "rgba(255, 255, 255, 0.10)"
  veil-pressed: "rgba(255, 255, 255, 0.14)"
  line: "rgba(255, 255, 255, 0.06)"
  line-input: "rgba(255, 255, 255, 0.10)"
  line-hover: "rgba(255, 255, 255, 0.18)"
  shade-input: "rgba(0, 0, 0, 0.32)"
  shade-input-hover: "rgba(0, 0, 0, 0.42)"
  scrim: "rgba(0, 0, 0, 0.60)"
typography:
  display:
    fontFamily: "Figtree, Segoe UI Variable Text, sans-serif"
    fontSize: "40px"
    fontWeight: 800
    lineHeight: "44px"
  headline:
    fontFamily: "Figtree, Segoe UI Variable Text, sans-serif"
    fontSize: "28px"
    fontWeight: 800
    lineHeight: "34px"
  title:
    fontFamily: "Segoe UI Variable Text, sans-serif"
    fontSize: "18px"
    fontWeight: 400
    lineHeight: "24px"
  body:
    fontFamily: "Segoe UI Variable Text, sans-serif"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: "20px"
  secondary:
    fontFamily: "Segoe UI Variable Text, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "18px"
  label:
    fontFamily: "Segoe UI Variable Text, sans-serif"
    fontSize: "12px"
    fontWeight: 500
    lineHeight: "16px"
    fontFeature: "tnum"
rounded:
  xs: "2px"
  sm: "4px"
  md: "6px"
  lg: "8px"
  xl: "12px"
  2xl: "20px"
  full: "9999px"
spacing:
  s0: "2px"
  s1: "4px"
  s2: "8px"
  s3: "12px"
  s4: "16px"
  s5: "24px"
  s6: "32px"
  s7: "48px"
  s8: "64px"
components:
  button-primary:
    backgroundColor: "{colors.cover-accent-default}"
    textColor: "{colors.night-room}"
    typography: "{typography.body}"
    rounded: "{rounded.full}"
    padding: "0 24px 0 16px"
    height: "40px"
  button-icon:
    backgroundColor: "transparent"
    textColor: "{colors.foreground}"
    rounded: "{rounded.full}"
    size: "36px"
  button-icon-hover:
    backgroundColor: "{colors.veil-hover}"
  button-icon-pressed:
    backgroundColor: "{colors.veil-pressed}"
  button-play:
    backgroundColor: "{colors.foreground}"
    textColor: "{colors.night-room}"
    rounded: "{rounded.full}"
    size: "44px"
  row:
    backgroundColor: "transparent"
    textColor: "{colors.foreground-muted}"
    typography: "{typography.secondary}"
    rounded: "{rounded.md}"
    padding: "0 12px"
    height: "56px"
  row-hover:
    backgroundColor: "{colors.veil-hover}"
  row-selected:
    backgroundColor: "{colors.veil-selected}"
  row-playing:
    # Kapak ışığının %8 tonu; üzerine gelince %12, seçiliyken %16, basınca %20.
    backgroundColor: "{colors.cover-accent-default}"
  search-field:
    backgroundColor: "{colors.shade-input}"
    textColor: "{colors.foreground-muted}"
    typography: "{typography.secondary}"
    rounded: "{rounded.lg}"
    padding: "0 12px"
    height: "32px"
  search-pill:
    # Başlık çubuğundaki arama: tam yuvarlak; dolgu kapak ışığıyla tonlu (opak, parlaklık tavanlı).
    backgroundColor: "{colors.cover-glow-default}"
    textColor: "{colors.foreground-soft}"
    typography: "{typography.body}"
    rounded: "{rounded.full}"
    padding: "0 16px"
    height: "36px"
  search-surface:
    # Ctrl+K yüzeyi: cam zemin (hapın camı), 20 px köşe, 720 px.
    backgroundColor: "{colors.bar-surface}"
    textColor: "{colors.foreground}"
    typography: "{typography.body}"
    rounded: "{rounded.2xl}"
    padding: "8px"
  kbd:
    backgroundColor: "{colors.veil-selected}"
    textColor: "{colors.foreground-muted}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 8px"
  menu:
    backgroundColor: "{colors.bar-surface}"
    rounded: "{rounded.xl}"
    padding: "4px"
  menu-item:
    textColor: "{colors.foreground}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 8px"
    height: "32px"
  tooltip:
    backgroundColor: "{colors.bar-surface}"
    textColor: "{colors.foreground}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "4px 8px"
  button-primary-small:
    # Kayınca küçülen liste başlığındaki Çal / Duraklat.
    backgroundColor: "{colors.cover-accent-default}"
    textColor: "{colors.night-room}"
    typography: "{typography.secondary}"
    rounded: "{rounded.full}"
    padding: "0 16px 0 12px"
    height: "32px"
  chip:
    # "Tümü"deki tür süzgeci; seçili olan ön plan renginde dolu.
    backgroundColor: "{colors.veil-hover}"
    textColor: "{colors.foreground-muted}"
    typography: "{typography.secondary}"
    rounded: "{rounded.full}"
    padding: "0 12px"
    height: "32px"
  chip-selected:
    backgroundColor: "{colors.foreground}"
    textColor: "{colors.night-room}"
  shelf-card:
    # Raftaki liste: kapak + tek satır ad. Seçili kart kapak ışığı renginde halka ve ad.
    # Kartlar arası en az 16 px; raf doluyken artan yer aralara dağılır.
    # (7 Kasım gecesi: kapak 112'den 136'ya.)
    coverSize: "136px"
    rounded: "{rounded.lg}"
    typography: "{typography.secondary}"
    gap: "8px"
    cardGap: "16px"
    height: "182px"
  shelf-strip:
    # Liste kayınca raf bu şeride iner.
    coverSize: "40px"
    rounded: "{rounded.sm}"
    height: "56px"
    gap: "8px"
  library-card:
    # Kütüphane görünümündeki ızgara kartı (7 Kasım gecesi): kapak hedefi 168 px
    # (sütuna göre 144-200), kartlar ve satırlar arası 24 px.
    coverSize: "168px"
    rounded: "{rounded.lg}"
    typography: "{typography.secondary}"
    gap: "24px"
  ghost-button:
    # Kütüphane araç satırı (sıralama, Yeni liste): kenarsız, saydam zemin.
    backgroundColor: "transparent"
    textColor: "{colors.foreground-muted}"
    typography: "{typography.secondary}"
    rounded: "{rounded.full}"
    padding: "0 12px"
    height: "32px"
  hero-card:
    # Ana sayfa kahramanları (7 Kasım gecesi): Kaldığın yerden + günün önerisi.
    # Kartta küçük üst etiket yok (12 Kasım, eyebrow yasağı): ad, alt yazı, düğme.
    # Zemin kapak renginden üstten alta koyulaşan ton (en parlak yeri göreli
    # parlaklık 0,04'ü geçmez: yazı her renkte okunur).
    backgroundColor: "{colors.bar-surface} + kapak rengi"
    rounded: "{rounded.xl}"
    padding: "24px"
    height: "232px"
    coverSize: "184px"
    pickCoverSize: "136px"
  list-header:
    # Açıkken iki satır: ad (Headline), altında meta (Body); Çal ve diğer düğmeler bloğun dikey ortasında. Kayınca tek satıra iner.
    typography: "{typography.headline}"
    metaTypography: "{typography.body}"
    height: "70px"
    compactTypography: "{typography.title}"
    compactHeight: "48px"
  page-header:
    # Sanatçı, albüm, ana sayfa, arama: aynı başlık bloğu.
    artSize: "160px"
    artSizeCompact: "128px"
    typography: "{typography.display}"
    compactTypography: "{typography.headline}"
    metaTypography: "{typography.body}"
    gap: "24px"
    # Üst boşluk (12 Kasım): kapaksız sayfalar (Keşfet, Ayarlar, Araçlar, Arama,
    # Son çalınanlar) 48 px (Ana sayfadaki selamlamayla aynı), kapaklı (sanatçı,
    # albüm) 24 px; yükleniyor iskeleti aynı boşluğu taşır.
    paddingTop: "48px"
    paddingTopWithArt: "24px"
  page-card:
    # Sayfalardaki kart ızgarası: tam sütun, kart en az 152 px, sütuna göre esner.
    minWidth: "152px"
    gap: "16px"
    rounded: "{rounded.lg}"
    typography: "{typography.body}"
  section-title:
    typography: "{typography.title}"
    fontWeight: 800
    height: "48px"
  toast:
    # Geri al bildirimi: sağ altta, hapın üstünde.
    backgroundColor: "{colors.bar-surface}"
    rounded: "{rounded.xl}"
    height: "48px"
    typography: "{typography.secondary}"
  sync-badge:
    height: "32px"
    rounded: "{rounded.full}"
    typography: "{typography.label}"
  player-capsule:
    # Ortada yüzen koyu cam (7 Kasım): zemin #19191D'den #0F0F12'ye (üstten
    # alta), çalarken kapağın bulanık renklerinden soldan yayılan ışık
    # (en parlak yeri göreli parlaklık 0,05), üstte ışık çizgisi (beyaz %7,
    # halkayla üstte ~%12), 1 px iç halka (beyaz %5,5), renkli kenar yok.
    backgroundColor: "#19191D → #0F0F12"
    rounded: "{rounded.full}"
    height: "72px"
    maxWidth: "800px"
    padding: "0 16px 0 12px"
    coverSize: "48px"
    bottom: "24px"
  toggle-button:
    # Karıştır, tekrar, sıra, sözler: simge düğmesi; açıkken simge kapak ışığı
    # renginde ve altında 4 px nokta.
    backgroundColor: "transparent"
    textColor: "{colors.foreground}"
    rounded: "{rounded.full}"
    size: "36px"
  side-panel:
    # Sağ panel: Şimdi çalıyor / Sıradaki / Sözler. 1720 px ve üstü pencerede
    # listenin yanına yerleşik (sabit 400 px, sağ kenardan 16 px; liste
    # solundaki bütün yeri alır), daha darda içeriğin üstünde yüzer (360 px).
    backgroundColor: "{colors.bar-surface}"
    rounded: "{rounded.xl}"
    width: "360px"
    dockedWidth: "400px"
    right: "16px"
    headerHeight: "56px"
    dockMinWindow: "1720px"
    coverSize: "326px"
    dockedCoverSize: "366px"
  panel-section-title:
    # "Şimdi çalıyor" sekmesinde bölüm başlığı (Sıradaki, Sözler, Sanatçı,
    # Son çalınanlar): sayfalardaki bölüm başlığının panel boyu. Bölümler
    # arası 24 px, başlıkla içerik arası 8 px.
    typography: "{typography.body}"
    fontWeight: 700
    textColor: "{colors.foreground}"
  queue-row:
    backgroundColor: "transparent"
    textColor: "{colors.foreground-muted}"
    typography: "{typography.secondary}"
    rounded: "{rounded.md}"
    height: "56px"
    coverSize: "40px"
  lyrics-line:
    # Panelde Title (Figtree 700), tam ekranda Headline (Figtree 800).
    textColor: "{colors.foreground-muted}"
    activeTextColor: "{colors.foreground}"
    typography: "{typography.title}"
  switch:
    # Ayarlar (Adım 15): açık / kapalı. Kapalı: oluk rengi (track-groove) ve soluk
    # düğme; açık: kapak ışığı rengi ve gece odası renginde düğme. Üzerine gelince
    # %6 / %10 ton ya da (açıkken) vurgunun %16 açığı.
    backgroundColor: "{colors.track-groove}"
    activeBackgroundColor: "{colors.cover-accent-default}"
    rounded: "{rounded.full}"
    width: "44px"
    height: "24px"
    knob: "20px"
    padding: "{spacing.s0}"
  segmented:
    # Birkaç seçenekten biri (ses kalitesi, animasyon, kapat düğmesi): siyah %32 zemin,
    # beyaz %10 kenar, içinde 28 px'lik seçenekler; seçili olan ön plan renginde dolu.
    backgroundColor: "{colors.shade-input}"
    rounded: "{rounded.full}"
    padding: "{spacing.s0}"
    segmentHeight: "28px"
    segmentPadding: "0 12px"
    typography: "{typography.secondary}"
  select-field:
    # Çıkış aygıtı: arama kutusu gibi (siyah %32, beyaz %10 kenar, 8 px köşe), sağda
    # aşağı ok; açıkken kenar kapak ışığı renginde, liste menü paneliyle altında açılır.
    backgroundColor: "{colors.shade-input}"
    rounded: "{rounded.lg}"
    height: "32px"
    width: "280px"
    typography: "{typography.secondary}"
  profile-button:
    # Başlık çubuğunun en solunda hap: 24 px avatar (baş harf ya da kişi simgesi;
    # Spotify profil fotoğrafı oturum verisinde yok) + ad + aşağı ok; %6 ton zemin.
    backgroundColor: "{colors.veil-hover}"
    rounded: "{rounded.full}"
    height: "32px"
    margin: "0 0 0 24px"
  settings-row:
    # Ayarlar sayfasında bir satır: solda ad (Body, yarı kalın) ve tek cümle açıklama
    # (Secondary, soluk), sağda denetim. Satırlar arası %6 çizgi, kutu yok.
    minHeight: "64px"
    padding: "12px 0"
    gap: "24px"
    maxWidth: "720px"
---

# Design System: Openify

## Overview

**Creative North Star: "Sakin dinleme odası"**

Openify sakin bir dinleme odası gibi davranır: koyu temada nötr gece, açık temada sıcak kırık beyaz yüzeyler geri çekilir; renk çalan şarkının kapağından gelir. Vurgu ve pencerenin üstündeki ton kapaktan türetilir ve şarkı değişince yavaşça (0,8 sn) yeni renge geçer. Arayüzün geri kalanı bu rengi taşımak için var; kendisi göze girmez.

Yön, ağırlık sırasıyla Sade > Atmosferik > Editoryal. Sade taban: nötr siyah zemin, sıkı ama nefes alan satırlar, altta yüzen çalar. Atmosferik katman: kapaktan gelen renk ve ışıma, her yerde kapaklar. Editoryal dokunuş: sayfa başlıklarında kalın, karakterli Figtree. Yüzeyler gölgeyle değil, beyazın saydam ton katmanlarıyla ayrılır; gölge yalnızca içeriğin üstünde yüzen geçici öğelerde.

Bileşenlerin hissi "sakin ve kesin": dururken neredeyse görünmezler, dokununca anında ve net tepki verirler. Bu bir çalardır; dikkat müzikte kalır.

**Key Characteristics:**
- Tek ışık kaynağı: renk yalnızca kapaktan gelir, arayüzün kendi rengi yok.
- Ton katmanlarıyla derinlik; gölge yalnızca yüzen öğelerde.
- Her blok tek bir sol kenara dizilir; raf, liste başlığı ve satır içeriği aynı kenardan (sayfa kenar boşluğu: 32 / 48 / 64 px, `page_margin`) başlar.
- Kapaklar kahraman: kütüphane bir kapak rafı, çalar kapağın ışığını taşıyan yüzen bir hap.
- Altı basamaklı yazı ölçeği, dokuz basamaklı boşluk ölçeği; kodda ölçek dışı değer yok (test ile korunur).
- Her etkileşimli öğenin beş durumu tasarlı: normal, üzerine gelme, basma, klavye odağı, pasif.

## Colors

Koyu temada nötr gece zemini ve açık temada sıcak kırık beyaz zemin; ikisinde
de kapaktan gelen tek vurgu. Renklerin kaynağı `theme.rs` içindeki `Palette`;
yüzeyler ve mürekkep birlikte seçilir, açık tema renk tersine çevirmesi değildir.

### Açık palet ve tema seçimi

| Jeton | Koyu | Açık |
|---|---|---|
| Pencere | `#0E0E10` | `#F7F4EE` |
| Yüzen yüzey | `#121215` | `#FFFCF7` |
| Kenar | `#1C1C21` | `#DBD5CB` |
| Metin | `#F4F4F5` | `#24211F` |
| Oluk | `#2A2A30` | `#D5CEC3` |
| Kapak yer tutucu | `#1C1C21` | `#E7E0D5` |
| Tehlike metni | `#F87171` | `#AD2727` |
| Hap camı, üst → alt | `#19191D` → `#0F0F12` | `#FFFFFB` → `#ECE7DF` |

- **Sıcak yüzey, gerçek mürekkep:** açık temada ikincil metin %80, etiket %78
  koyu mürekkep; koyu temada ikincil %72, etiket %68. Bu değerler seçili ve
  basılı satırlarda da karşıtlığı korur. Ana metin %100, simgeler %86. Pasif simgeler okunacak metin
  yerine kullanılmaz. Ana metin / yüzey çiftleri iki temada WCAG AA ≥ 4,5:1
  birim testiyle korunur.
- **Kapak rengi kalır:** açık temada pencere ışıması kapak renginin %8'i,
  arama hapı %7 (açıkken %10), Home kartı üstte %9 / altta %3 ton alır.
  Vurgu açık zeminde koyulaştırılır; tonu korunur. Asıl düğmenin yazısı açık
  mürekkeptir; seçili çipler koyu mürekkep dolgulu ve açık yazılıdır.
- **Cam ve tam ekran:** hapın renkli lekesi en çok %14, tam ekranın bulanık
  kapağı %10 karışır; göreli parlaklık en az 0,68 kalır. Arka planda hazırlanan
  dokular tema değişince mevcut kapaktan yeniden üretilir; eski işin sonucu
  yeni temanın üstüne gelemez.
- **Yumuşak derinlik:** açık temada gölge sıcak kahverengi ve koyu gölgenin
  %28 yoğunluğundadır. Giriş gölgelendirmesi koyu mürekkebin %16 gücüne iner;
  hapın alt perdesi pencerenin kırık beyazına söner. Kapak üstündeki oynat,
  büyüt ve sabitle işaretleri kendi koyu perdesi ve beyaz mürekkebini korur.
- **Ayar:** Görünüm ve davranış → Tema: Koyu / Açık / Sistem. Eksik veya
  bilinmeyen kayıt Koyu'dur. Sistem, Windows'un uygulama renk modunu izler
  (`AppsUseLightTheme`, salt okuma); değişim yeniden başlatmadan uygulanır.
  Başlık çubuğu DWM bayrağı ve renkleri de aynı palete geçer. Tepsi ve görev
  çubuğu simgeleri Windows'un ayrı kabuk modunu izlemeyi sürdürür.

### Primary
- **Kapak Işığı** (çalan şarkıdan; varsayılan `cover-accent-default`): Çalan satırın adı, ekolayzer, asıl düğme, odak halkası, raftaki seçili kartın halkası ve adı, çalan listenin kapağındaki ses simgesi. Zemine ve ışımanın en parlak yerine göre her zaman ≥ 4,5:1 olacak kadar açılır, tonu korunur.
- **Kapak Işıması** (`cover-glow-default`): Pencerenin üstündeki 460 px'lik ışıma. Zeminle önceden karıştırılır (%36) ve zeminin kendisine erir; saydam degrade kullanılmaz. En parlak yeri göreli parlaklık 0,025'i geçmez: sarı ya da pembe bir kapak da odayı gece bırakır ve üstündeki etiketler 4,5:1'in altına düşmez.

### Neutral
- **Gece Odası** (`night-room`): Pencerenin zemini.
- **Çubuk Yüzeyi** (`bar-surface`): Yüzen çalar hapı ve yüzen paneller (palet, menü, ipucu).
- **Sessiz Kenar** (`quiet-edge`): Yüzen panellerin ve bölgelerin kenarı.
- **Ön Plan** (`foreground`): Bütün yazılar bu rengin saydam tonlarıdır: ana metin tam; ikincil ve etiket koyu temada %72 / %68, açık temada %80 / %78; pasif simge %28. Gri değil ön plan tonu kullanılır; kapak tonunun ve etkileşim katmanlarının üstünde okunabilir kalır.
- **Oluk** (`track-groove`): İlerleme ve ses çubuklarının boş kısmı.
- **Tehlike Yazısı** (`danger-text`): Silme ve çıkarma gibi geri dönüşü zor işlerin yazısı.

### Named Rules
**Tek Işık Kuralı.** Arayüzün kendine ait bir vurgu rengi yoktur. Renk yalnızca çalan şarkının kapağından gelir; kapak yoksa varsayılan turuncu. Henüz hiçbir şey çalmamışken oda karanlıktır (ışıma yok); ışık ilk şarkının kapağıyla, renk geçişiyle yanar. Denetimlerin yazı vurgusu (Çal düğmesi, odak halkası) karanlıkta da varsayılan turuncudur.

**Gece Kalır Kuralı.** Kapak ne kadar parlak olursa olsun ışıma bir tavanı aşmaz; üstündeki en soluk yazı (etiket) her renkte ≥ 4,5:1 kalır (test ile korunur).

**Ton Katmanı Kuralı.** Aynı durum her yerde aynı katmanla gösterilir: üzerine gelme %6, seçili %10, basma %14 (beyaz). Çalan satır beyaz değil kapak ışığının tonunu alır ve hangi durumda olursa olsun onu korur: çalıyor %8, üzerine gelince %12, seçiliyken %16, basınca %20. Bir ekran kendi saydamlık değerini uyduramaz.

## Typography

**Display Font:** Figtree (gömülü, OFL; Segoe UI Variable Text'e düşer)
**Body Font:** Segoe UI Variable Text (Windows'un arayüz yazı tipi, küçük boyutta ipucu verisiyle net)

**Character:** Başlıklarda Figtree'nin kalın, yuvarlak karakteri kişilik verir; geri kalan her şey Windows'un kendi yazısıyla sakin ve keskin. Figtree 12–14 px'te bulanık çizildiği için küçük yazıda kullanılmaz.

### Hierarchy
- **Display** (800, 40 px, 44 px satır): Boş ekrandaki "Openify".
- **Headline** (800, 28 px, 34 px): Liste başlığındaki ad ("Beğenilenler"), "Tümü" karosundaki sayı, bölüm başlıkları.
- **Title** (400–800, 18 px, 24 px): Palet girişi; kayınca küçülen liste başlığında ad (800, Figtree).
- **Body** (500, 14 px, 20 px): Şarkı adı, liste adı, menü öğesi.
- **Secondary** (400, 13 px, 18 px): Sanatçı, meta satırı, açıklama.
- **Label** (500–600, 12 px, 16 px, eşit genişlikte rakam): Sütun başlığı, sıra panelindeki grup adı, süre, kısayol. (Okunacak bir bölümün başlığı Label değildir: sayfalarda Title, panelde Body kalın.)

### Named Rules
**Altı Basamak Kuralı.** Kodda bu altı rolün dışında yazı boyutu yoktur; boyut ve satır yüksekliği yalnızca `type_role` ile verilir.

**Titremeyen Rakam Kuralı.** Değişen her sayı (süre, konum, sayaç) eşit genişlikte rakamla yazılır.

## Layout

**Raf + yüzen hap** (6 Kasım kararı). Kenar çubuğu yok. Yukarıdan aşağı: 44 px başlık çubuğu (solda ad, ortada 420–520 px hap biçimli arama, sağda pencere düğmeleri), kapak rafı, tek satırlık liste başlığı, liste; çalar listenin üstünde yüzer. **Sayfa kenar boşluğu** (8 Kasım, kullanıcı geri bildirimi: liste kenardan kenara uzuyordu; tek yardımcı `theme::page_margin(içerik_eni)`): içerik alanı 1280 px'ten darsa 32, 1280'den 48, 1600'den 64 px; sol ve sağ eşit. Raf, liste başlığı, sütun başlıkları, satırlar, Bul çubuğu, "Tümü" ızgarası aynı değeri kullanır, kenarlar hizalı kalır (içerik eni = pencere eni; yerleşik panel açıkken panelin kapladığı yer çıkmış hâl). 1180'de 32 (eskisi gibi), 1920'de panel kapalıyken 64, panel yerleşikken 48. Eşikler içeriğin yaklaşık %4–5'ini kenar boşluğuna ayırır: geniş ekranda liste çerçeve içinde durur ama ölü boşluk oluşmaz. Ortak sağ kenar aynı değer (yerleşik panel açıkken panelin solunda): rafın son kartı ("Tümü"), liste başlığındaki Çal düğmesi, "Süre" başlığı ve süreler aynı kenarda biter.

- **Raf** (182 px): son çalınana göre sıralı listeler, 136 px kapak (7 Kasım gecesi; 112'den: kullanıcı "kütüphane yorucu, kapaklar küçük" dedi. Gerekçe: 1180 px'te (iç en 1116) tam yedi kart sığar, artan 68 px aralara dağılır ve aralar ~28 px olur, raf nefes alır; 144 de yedi kart sığdırırdı ama aralar 18 px'e düşer ve raf 8 px daha uzar; 128'de 1180'de aralar ~35 px olurdu. Bedel: liste 24 px aşağı iner, 1180×760'ta raf açıkken hapın üstünde 6,4 yerine ~5,8 satır görünür; çıta 5,7'ye çekildi, kayınca 8,6 aynı) + tek satır ad, aralarında en az 16 px. Pencereye sığdığı kadar kart görünür (yatay kaydırma yok), sonda "Tümü · N" karosu. Raf doluyken artan yer aralara eşit dağılır: "Tümü" karosu sağ sayfa kenarına, liste başlığındaki Çal düğmesinin ve süre sütununun hizasına oturur (7 Kasım, kullanıcı geri bildirimi; önce sağda 0–111 px boş kalıyordu). Kütüphane rafı doldurmuyorsa kartlar soldan 16 px arayla. Sıra: beğenilenler hiç çalınmadıysa en önde, sonra son çalınanlar (en yenisi önce), sonra kütüphanenin geri kalanı kütüphane sırasıyla. "Son çalınan", bir liste çalınmaya başlanınca (satır, başlıktaki Çal, haptaki Çal, palet) kaydedilir ve ayarlarda saklanır (en çok 50). Açık liste rafta yoksa (Tümü'den ya da aramadan açıldı) son yerde durur: nerede olunduğu rafta hep görünür. Seçili kart: kapak ışığı renginde halka ve ad.
- **Şerit** (56 px): liste aşağı kayınca raf, listeyle aynı hareketle şeride iner: kapaklar 136'dan 40 px'e küçülür, aralar 16'dan 8 px'e daralır, adlar yarı yolda solar; sonda "Tümü · N" yazılı düğme. Kapaklar tıklanabilir kalır. Liste en üste dönünce raf açılır. Raf ve başlığın kaybettiği yükseklik tekerleğin aldığı yola eşittir (liste kaymadan önce raf ve başlık kapanır, satırlar zıplamaz). Başka bir listeye geçince raf olduğu hâlde kalır (şeritteki kapağa tıklayınca raf farenin altında açılmaz); liste başa döner. Rafın üstünde tekerlek de listeyi kaydırır.
- **Tümü / Kütüphane görünümü:** bütün kütüphane, rafın altında kapak ızgarası (yalnız görünen satırlar çizilir). 7 Kasım gecesi sakinleştirildi: kart hedefi 168 px (sütun sayısı bu hedefe en yakın tam sayı, kartlar sütuna göre esner: 1180'de 6 × 166, 1920'de panel yerleşikken 7 × 181, 760'ta 4 × 156), sütun ve satır araları 24 px (önce 112 px kart, 16 px), son sütun sağ kenara (kapatma düğmesinin sağ kenarına) oturur. **Rafı tekrarlamaz:** sıralama "Son çalınan", süzgeç ve yazı yokken raftaki kartlar ızgaranın başında bir daha çıkmaz (ızgara rafın bıraktığı yerden başlar; raftan fazlası yoksa ya da süzülünce / "Ad" sıralamasında hiçbir şey atlanmaz). **Araç satırı hafif:** süzme kutusu boşken kenarsız ton zemini (yazılırken kenar kapak ışığı), türe göre haplar (zaten kenarsız), sağda sıralama ve Yeni liste "sessiz düğme" (`ghost-button`: kenarsız, simge + soluk yazı), ızgara / liste anahtarı; "n / N" sayacı yalnız süzülürken. **Araç satırı eşiği 1180 px** (12 Kasım; 900'de "Yeni liste" kırpılıyor, "+" ızgaranın sağ kenarından ~17 px taşıyordu; `ALL_COUNT_MIN_WIDTH`): 1180'in altında Araçlar ve Yeni liste yalnız simge (ipucu var) ve "n / N" sayacı gizli, 1180 ve üstünde yazılar görünür. **Raf kartı ipucu kuralı:** kartın adı yalnız şerit görünümünde (ad gizliyken, `name_opacity < 0,5`) ipucunda görünür; ad görünürken ikinci kez ipucu yok (gereksiz; kısaltılan ad `line_with_tip` ile zaten kendi ipucunu taşır). **Kapağı olmayan liste:** önbellekteki ilk şarkılarının dört farklı albüm kapağından 2×2 mozaik (aralarında 2 px, yalnız dış köşeler yuvarlak; engine `with_mosaics`, ağ yok), 1–3 farklı kapak varsa ilki, şarkıları önbellekte yoksa sakin yer tutucu (üstten alta hafif koyulaşan ton, küçük soluk simge). Açılınca yazmaya başlanır, yazdıkça süzülür (bulanık arama, Türkçe harfler sadeleşir); türe göre haplar (Hepsi / Albümler / Listeler / Spotify'ın sana özel); sağda "gösterilen / toplam" ve kapatma düğmesi. Yazarken Enter'ın açacağı ilk kart halkayla belli; Enter ilkini açar, Esc kapatır. Boş kutuda Boşluk yine çal / duraklat. Çalar yerinde kalır, üst ışıma yarıya söner.
- **Liste başlığı** (70 px, üstünde ince çizgi; 8 Kasım yeniden: meta ad yanında küçük ve sıkışık duruyordu): iki satırlı blok: ad (Headline), hemen altında meta satırı (Body 14 px; öndeki sahip / sanatçı adı ön plan renginde, yarı kalın, tıklanabilir yerlerde tıklanabilir; ` · ` ve gerisi ikincil renk), sağda bloğun dikey ortasında (beğen / diğer) ve "Çal" / "Duraklat" (asıl düğme). Büyük kapak yok (kapak rafta). Raf şeride inerken başlık da 48 px'e iner ve tek satıra döner (ad ve meta yan yana, meta Secondary); yarı yoldan sonra ad Title'a (18 px, Figtree 800), düğme 32 px'lik küçük asıl düğmeye geçer (yeni hâl belirerek gelir).
- **Satırlar:** 56 px; içerik alanının bütün enini kullanır (iki yanda sayfa kenar boşluğu, 32–64 px; yerleşik panel açıkken panelin solundaki alan): sütunlar #, (kapak), Şarkı, Albüm, kalp, Süre; süre sağ sayfa kenarında, şarkı ve albüm sütunları aradaki yeri her pencere eninde 4'e 3 paylaşır. Sütun başlıkları, liste başlığı (ad, meta, Çal) ve raf aynı iki kenara dizilir. Albüm sayfasında kapak ve albüm sütunu gizlenir. **Tam en kuralı (7 Kasım, kullanıcı geri bildirimi):** eskiden liste 1280 px'te duruyordu (`CONTENT_MAX`): geniş pencerede ve panel kapanınca sağda 300–600 px boş kalıyordu ("panel kapatınca sağda çok boşluk kalıyor"). Artık içerik alanında üst sınır yok; sayfalardaki kart ızgarası da aynı (aşağıda). "Eklenme tarihi" sütunu yok: şarkı satırlarımızda (`TrackRow`) eklenme anı tutulmuyor (Spotify'ın koleksiyon / liste kayıtlarında var, okunup atılıyor); taklit edilmez, veri saklanınca geniş pencerede eklenir.
- **Yüzen hap (çalar):** her zaman pencerenin ortasında (7 Kasım, kullanıcı geri bildirimi: yerleşik panel açılınca hapın sola kayması kötü duruyordu; panel açılıp kapanınca hap yerinden oynamaz), alttan 24 px yukarıda, 72 px yüksek, en fazla 800 px (Adım 11'de 720'den; dar pencerede pencere eksi iki yanda 32 px; yerleşik panele 24 px'ten fazla yaklaşacak olsa daralır, dar hap kuralları o enle işler: 400 px'lik panelle 1680 px'ten geniş her pencerede tam enli kalır, eşikte (1720) panelle arası 44 px). Solda 48 px yuvarlak kapak (tıklayınca tam ekran; üzerine gelince ortasında simge), ad / sanatçı (bildirim sanatçının yerinde; hiçbir şey çalmıyorken bildirim, çevrimdışı ya da "Hiçbir şey çalmıyor" + başlatılacak liste), **orta sütun iki satır** (13 Kasım, dış inceleme: ilerleme çubuğu düğmelerin yanındaki artan yere sığıyordu, 800 px'lik hapta ~60 px'ti ve sarma kullanılamıyordu): üstte karıştır / önceki / çal / sonraki / tekrar, altında 20 px'lik satırda süre yazıları ve ilerleme çubuğu (sütunun bütün eni: 800 px'lik hapta ~280 px, en dar hapta (696) ~250 px; 4,6 kat). Seçim gerekçesi: çubuğu hapın üst kenarına çekmek (tam enli ince çizgi) en geniş çubuğu verirdi ama süre yazıları (geçen / toplam) yuvarlak uçların yüzünden çizgiyle birlikte duramıyor, süreler yalnız üzerine gelince görünürdü; bilgi kaybı ve ikinci bir "üzerine gelince" durumu. İki satır süreleri sürekli gösterir, tek satır düğme düzenini korur ve çubuk hapla birlikte daralır (taşma yok). Düğme satırı 44 px (çal) + ilerleme satırı 20 px = 64 px, hapın 72 px'ine 4 px boşlukla sığar (düğmeler hapın ortasından 10 px yukarıda; kapak ve ses ortada). Çubuk 4 px, tıklama alanı 20 px (düğmelerin alanına taşmaz), tıklayınca sarar, sürükleyince sürekli sarar, üzerine gelince tutamaç belirir (eskisiyle aynı). Şarkı yokken ilerleme gizli ama yerinde; solda bildirim ya da çevrimdışı durumu varken çubuk yok ama satırı yerinde (düğmeler kaymaz). Süre yazıları her hap enliğinde görünür. Sıra ve sözler paneli düğmeleri, sessize al + ses çubuğu (hap 760 px'ten darsa, yani pencere 824 px'ten darsa, ses çubuğu gizlenir: sessize al kalır, üstünde tekerlek sesi değiştirir; şarkı adı sığsın). Yüzeyi koyu cam (7 Kasım, `pill_glass`): üstten alta hafifçe koyulaşan zemin, üstte yuvarlak uçlarda incelerek sönen ince ışık çizgisi, 1 px'lik soluk iç halka; renkli kenar çizgisi yok. Bir şarkı varken kapağın durduğu soldan kapağın bulanık renkleriyle yumuşak ışık yayılır (Apple Music'in mini çalarındaki gibi; kapağın ortasından elips biçiminde, hapın %60'ında söner): renkler kapağın renk gücüyle doyar (bulanıklık soldurduğu için belirgin artırılır), en parlak yeri göreli 0,05 (odanın tavanının iki katı: 0,025'te ışık camda zor seçiliyordu; hap yüzen bir yüzey, ışığın düştüğü yerde etiket tonunda yazı yok), siyah kapak ışık vermez (sakin cam), gri ya da beyaz kapak nötr, gümüş ışık verir; ışığın düştüğü yerde en soluk yazı ikincil (%64; hapta süren işin bildirimi de ikincil tonda) ve her kapakta ≥ 4,5:1, etiket tonundaki süreler ışığın söndüğü yerde ≥ 4,5:1, haptaki vurgu ışığa göre açılır (test). Işık şarkı başına bir kez ayrı iş parçacığında hazırlanan 800 × 72'lik bir resim; şarkı değişince yenisi eskisinin üstünde 0,8 sn'de belirir (vurgu rengiyle aynı ortam geçişi; "animasyonları azalt"ta anında), eskisi bellekten atılır. Hiçbir şey yokken ışıksız, sakin cam. Kaldığı yerden açılan şarkı çevrimdışıyken sanatçının yerinde soluk wifi-off simgesi + "Çevrimdışı · Kütüphane önbellekten"; şarkı çalınamadığı için ilerlemenin yerini de şarkı bilgisi alır (boştaki çevrimdışı hâl gibi; metin 76 px'e sığmıyordu). Bağlantı gelince ilerleme yerine döner. Yüzdüğü için yüzen gölge alır ve altındaki satırları örter (boş yerine tıklamak arkadaki satırı seçmez; tekerlek yine listeyi kaydırır). **Alt perde** (13 Kasım, dış inceleme: son satırlar hapın altında kesik görünüyordu): pencerenin alt kenarından yukarı hap payı + 32 px (`PILL_CLEARANCE + PILL_SCRIM_FADE`) yükselen siyah degrade, üstte saydam, altta %50 (`PILL_SCRIM_ALPHA`; `shade`, ham renk yok); içeriğin (ve ekolayzerin) üstünde, hapın altında; tıklamayı ve tekerleği etkilemez; hap yokken perde de yok. Hapın yanından ve altından geçen satırlar hafifçe söner, hap öne çıkar; hapın üst kenarında perde ~%12 olduğundan hapın hemen üstündeki satır okunur kalır. **Liste sonu payı:** kaydırılan her yüzeyin sonunda hap boyu + 24 px pay (`PILL_CLEARANCE`) vardır; şarkı listesinde ek 16 px nefes boşluğu (`PILL_LIST_END`): en sona kaydırınca son satır tamamen görünür ve hapın üst kenarının 16 px üstünde durur (ölçüldü: 1180×760'ta 28. satır hapın üstünde 16 px boşlukla; sanatçı, albüm, ana sayfa, ayarlar sayfaları en sonda ≥ 24 px boşlukla). Kayarken satırlar hapın altından (ve iki yanından) geçer; çalan satırın ekolayzeri de hapın altında kalır.
- **Işık:** üstte rafın arkasına yayılan kapak ışıması (raf şeride indikçe yarıya söner), hap ve çalan satır kapak renginde; boştayken karanlık oda.
- **Yoğunluk çıtası (1180×760):** raf açıkken hapın üstünde ≥ 5,7 satır (7 Kasım gecesi rafın 136 px'e büyümesiyle 6,4'ten indi, kullanıcının kararı), kayınca 8 tam ve 9.'nun çoğu (8,7) (test ile korunur).
- **Küçük pencere (760×500'e kadar; kullanıcı onayı bekliyor):** pencere 640 px'ten kısaysa raf şerit hâlinde, başlık küçük açılır (yukarı kaydırmak yine açar); 760×500'de hapın üstünde 4 şarkı. Hap pencereyle daralır (pencere eksi 64 px); içinde şarkı bilgisi 112 px'e, ses çubuğu 40 px'e kadar daralır; orta sütun (düğmeler + ilerleme) en az 204 px, ilerleme çubuğu bu enle bile ≥ 116 px kalır. 1000 px'ten dar pencerede arama 280 px. Rafa sığan kart sayısı pencere genişliğinden (760 px'te 5 liste + Tümü). Hiçbir şey başlık çubuğunun ya da hapın üstüne binmez.

- **Başlık çubuğunda gezinme (Adım 12):** solda geri, ileri ve ana sayfa simge düğmeleri (36 px); en solda profil hapı (24 px boşluktan), ardından geri / ileri. Düğmeler sürükleme alanının dışında (içinde olsalar Windows o noktayı başlık sayar). Gidilemeyen yön pasif. Sağda, pencere düğmelerinden önce yalnız bekleyen ya da yazılamayan değişiklik varken eşitleme göstergesi.
- **Sayfalar (sanatçı, albüm, ana sayfa, arama):** rafın ve listenin yerinde açılır (raf gizli, "Tümü" gibi), hap yerinde. Yukarıdan aşağı: başlık bloğu (üstte 48 px boşluk, kapaklı sayfada 24 px; 160 px kapak / portre, Display ad, meta Body, eylemler; dar pencerede 128 px ve Headline), sonra bölümler. Bölüm başlığı Title (18 px, Figtree 800), 48 px yüksek; uzun bölümde sağda "Tümünü göster" / "Daha az" hapı. Kart bölümleri önce tek satır (pencereye sığan sütun kadar) gösterir. Kart ızgarası her pencere eninde tam sütun: sütun sayısı en dar kart enine (152 px) göre, sonra kartlar esner (yarım kart yok); ızgara içerik alanının bütün enini kullanır, son sütun sağ sayfa kenarına oturur (7 Kasım; önce 1280 px'te duruyordu). Şarkı bölümleri aynı şarkı satırını kullanır (popüler şarkılar ve arama: önce 5). Kütüphanendeki albüm liste görünümünde açılır (rafta yeri belli), başka albüm sayfada. Benzer sanatçılar şimdilik boş geliyor: boş bölüm hiç çizilmez. **Kenar boşluğu** sayfalarda da `theme::page_margin` (liste satırları ve raf ile aynı).
- **Açılış ve ana sayfa (7 Kasım gecesi yeniden: "klasik Spotify'a fazla benziyor, selamlama tepeye yapışık"):** uygulama ana sayfada açılır (geri / ileri geçmişi orada başlar; kaldığı yerden çalma aynı; Beğenilenler arkadan yüklenir). "Şimdi" odaklı. Yukarıdan aşağı: **selamlama** (günün saatine göre; Display; Günaydın 05–11, İyi günler 12–17, İyi akşamlar 18–22, İyi geceler; tepeden 48 px boşluk, altında dilime göre tek sakin Body satırı: kütüphane sayıları kalktı); **kahraman satırı** (232 px; iki kart yan yana %60 / %40, 900 px'ten dar içerikte alt alta; aralarında 24 px; kart: 12 px köşe, 24 px iç boşluk, zemini kapak renginden üstten alta koyulaşan ton, en parlak yeri göreli parlaklık 0,04'ü geçmez: beyaz yazı ≥ 10:1, ikincil ≥ 4,5:1, her renkte testli): solda **Kaldığın yerden** (184 px kapak, üstte etiket yok (eyebrow yasağı; 12 Kasım, önce küçük "Kaldığın yerden" / "Şimdi çalıyor" yazısı vardı): ad kapağın üst kenarından başlar, ad Headline, sanatçı Title, altta asıl düğme "Devam et" / "Duraklat" / "Baştan çal" ve yanında ilerleme çubuğu + süreler; çalarken saniyede bir yenilenir, yalnız liste görünümü; kapak tıklayınca tam ekran; renk çalan şarkının kapağından; hiçbir şey yüklü değilken son çalınan liste ve "Çal", o da yoksa Beğenilenler), yanında **günün önerisi** (üstte etiket yok (önce "Bu akşam için"); 136 px kapak, ad, açıklama, ikincil "Çal" dikeyde ortalı; kartın kendisi listeyi açar, sağ tık kart menüsü). Öneri deterministik: Spotify'ın kendi "daylist"i varsa o; yoksa ana sayfa akışındaki karışımların adında / açıklamasında günün diliminin sözcüğü en çok geçen (sabah workout / energy / upbeat, öğleden sonra focus / indie / pop, akşam chill / moody / jazz / lofi / sakin, gece sleep / ambient / piano / calm); yoksa günlük karışımlardan biri (`saat / 4` ile); akış yoksa kütüphanedeki Spotify listelerinden. Sonra **az sayıda bölüm**, her birinde başlığın altında tek açıklama satırı ve bir sıra kart (sayfa kartı kuralları, aralarında 48 px): **Çok dinlediklerin** (Spotify'ın "On Repeat" / "Repeat Rewind"ı varsa ve son çalınan listeler; kahramandakini tekrarlamaz), **Senin için yeni** (Haftalık Keşif, Yeni Müzik Radarı, daylist ve günlük karışımlar TEK yığın kart: üç kapak 12 px kayarak üst üste, "6 karışım", tıklayınca ilkini açar; "Tümünü göster" hepsini tek tek açar: günlük karışımlar, tür karışımları ve radyolar dahil, önce ayrı "Karışımların" / "Radyolar" satırlarıydı), **Listelerin** (kendi listelerin; Spotify'ın hazırladıkları ve yukarıda gösterilenler hariç). **Hızlı erişim ızgarası kalktı**: Beğenilenler ve son çalınanlar zaten kahraman kartta ve "Çok dinlediklerin"de, ızgara aynı şeyi ikinci kez gösteriyordu. **"Albümlerin" satırı kalktı** (Kütüphane sekmesinde, rafta ve ızgarada). Yalnız elimizdeki veri: Spotify'ın ana sayfa akışı (önbellekten, gelene dek iskelet, gelmezse kütüphane), kütüphane ve yerel çalma geçmişi.
- **Albüm sayfası (7 Kasım akşam):** başlıkta 224 px kapak (8 px köşe, kapak kutusunu doldurur; dar pencerede 160), yanında alta hizalı blok: Display ad, tıklanabilir sanatçılar (Title, yarı kalın, ön plan rengi), meta (Secondary: tür · yıl · şarkı sayısı · süre), Çal / kalp / diğer. Şarkı listesi başlığın altında kütüphane listesi gibi sayfa kenarından kenarına uzar (`theme::page_margin`, aynı sol ve sağ kenar; 1040 px üst sınırı kalktı, sağda boş alan bırakıyordu; sayfadaki bütün içerik kenarları `page_margin(window_width)`, sabit 32 yok). Albüm sayfasında kapak ve albüm sütunu yok. İçerik 1100 px'ten genişse ve albümde albüm sanatçısından farklı sanatçılı şarkı (feat. vb.) varsa sanatçılar adın altından ayrı bir "sanatçı" sütununa geçer (ortadaki boşluk gerçek veriyle dolar; sahte veri yok, çalma sayısı elimizde yok); tüm şarkılar albüm sanatçısınınsa sütun yok, ad solda, süre sağda, kalp satırın üzerine gelince (kütüphane listesindeki gibi). Dar içerikte farklı sanatçı adın altında kalır. Kapaktaki siyah çerçeve kimi albümlerin (ör. "666") kendi görselinin parçasıdır, çerçeve bizden değil (kapak dosyası 256 px, kenarlarında siyah şerit). En altta etiket satırı, sanatçı sayfası bellekteyse "<sanatçı> sanatçısından daha fazlası" kart satırı (albüm gelince sanatçı sayfası arkadan bir kez istenir).
- **Geri / ileri:** her yer (liste, sayfa) geçmişe eklenir; dönünce bırakıldığı kaydırma konumunda açılır. Alt+← / →, farenin yan tuşları, başlık çubuğundaki düğmeler.
- **Sayfa geçişi (7 Kasım):** yeni yerin (liste, sanatçı, albüm, ana sayfa, arama; geri / ileri dahil) içeriği belirerek ve 8 px aşağıdan yükselerek gelir: 220 ms, yavaşlayarak biter (`motion`, ease-out quint). Eski sayfa beklemez, içerik ilk karede yerinde ve tıklanabilir; geçiş bitince kare istenmez. İskeletten içeriğe geçişte yeniden oynamaz. "Animasyonları azalt"ta hiç hareket yok.
- **Hareket (7 Kasım akşam, kullanıcı geri bildirimi: "geçişler, animasyonlar daha premium ve akıcı hissettirmeli"):** bütün geçişler tek jeton tablosundan (`theme.rs`), her biri ease eğrisiyle; sürerken kare ister, biter bitmez istemez, girdiyi bekletmez, "animasyonları azalt"ta anında olur.

  | Jeton | Süre | Eğri | Nerede |
  |---|---|---|---|
  | `motion_fast` | 140 ms | ease-out kübik | üzerine gelme / basma rengi, sekme içeriği geçişi |
  | `motion` | 220 ms | ease-out quint (`quint_out`) | sayfa girişi, panel girişi, tam ekran girişi, palet |
  | `motion_exit` | 160 ms | ease-in kübik (`cubic_in`) | panel kapanışı, tam ekrandan dönüş |
  | `cover_fade` | 150 ms | ease-out kübik | inen kapağın belirmesi (yer tutucunun üstünde) |
  | `accent_fade` | 800 ms | ease-out kübik | ortam rengi / hap ışığı (şarkı değişince, çapraz geçiş) |

  Giriş yavaşlayarak biter, çıkış girişten kısa ve hızlanarak gider (kullanıcı kapattığı şeyin gitmesini beklemez). `OPENIFY_TEST_SLOW_MOTION=<kat>` hepsini uzatır (ara kare yakalamak için).
  - **Üzerine gelme / basma (`tween.rs` `HoverFill`):** gpui `.hover()` stilleri anında değişir; bütün düğmeler (simge, asıl, çip, ikincil, menü öğesi, hap düğmeleri), şarkı satırı, ana sayfa karosu, sayfa kartı (kapak üstü ton katmanı), kalp / kapat / geri al düğmeleri zeminin üstüne ve içeriğin altına çizilen mutlak konumlu bir katmanla 140 ms'de renk değiştirir; basınca anında net. Saydam zeminli öğede katman, hedef rengin bileşik sonucunu verecek farktır (`overlay`, testli). Bırakılan fare yarıda dönerse bulunduğu yerden döner. İlerleme pencere öğesi durumunda (görünüme not yok); hedefe varınca saat durur, boşta kare yok.
  - **Panel (açma / kapama, `now_panel::Slide`):** geniş pencerede yerleşik panel sağdan içeri kayar ve içerik alanını iter: içerik her karede yeni enine geçer (liste, raf, sayfa, hap yeniden ölçülür; zıplama yok), 220 ms quint; kapanırken 160 ms ease-in, panel çıkarken çizilmeyi sürdürür (`exiting`). Dar pencerede yüzen panel yerinde belirir (24 px + saydam, çıkışta tersi). İlk kare, pencere genişlik durumunun değişmesi ve "animasyonları azalt" anında atlar. Yarıda dönülürse kalan yolla orantılı süre. Kenar boşluğu (`page_margin`) eşiklerden önceki 64 px'te rampayla geçer: kayma ve pencere boyutlandırma sırasında kenar zıplamaz.
  - **Panel sekmeleri** (Çalıyor / Sıradaki / Sözler; yan paneldeki sekme adı `NowTab` = TR "Çalıyor", EN "Playing": üç çip 222 px'e iki dilde de sığsın diye kısa ad; başlık ve öteki yerlerde `NowPlaying` "Şimdi çalıyor" kalır): yeni içerik 140 ms'de %30 taban saydamlıktan belirir (arada boş kare yok).
  - **Tam ekran, geniş pencere (12 Kasım):** mantıksal genişlik ≥ 1800'de kapak 640 px, söz sütunu 860 px, sözler Display (40 px); altında kapak 480, söz sütunu 640, sözler Headline (28). 2560×1440'ta 480 px kapak ve 28 px yazı ekranın ortasında küçük bir ada kalıyordu.
  - **Tam ekran:** açılırken 220 ms quint belirir (`FullState::opened_at`), kapanırken 160 ms söner (`closing`); kapak küçültme / ölçekleme yok (gpui'de öğe dönüşümü yok; kaydırma ve ölçek yerine yalnız belirme).
  - **Sayfa geçişi:** yukarıdaki "Sayfa geçişi (7 Kasım)" kaydı geçerli. Eski sayfanın çıkış solması yok: görüntü kopyası alınamadığı için bekletmek gerekirdi (girdiyi bekletmeme kuralı); yeni içerik arka plandan belirdiği için geçiş eski sayfayı karartıp yeni sayfayı açan bir çapraz geçiş gibi okunur.
  - **Değişmeyenler:** raf şeride iner (kaydırma konumunun işlevi, ayrı süre yok), hap ışığı çapraz geçişi (800 ms), kapak belirmesi (150 ms), ekolayzer / sözün kayması.

Boşluklar yalnızca ölçekten: 2 / 4 / 8 / 12 / 16 / 24 / 32 / 48 / 64. Yakın ilişkili öğeler küçük, ayrı gruplar büyük basamakla ayrılır; 2 px yalnızca aynı öğenin iki satırı arasında ve sıkı listelerde.

Uzun her yazı tek satırda "…" ile kısalır; kısaltılan yazının tamamı ipucunda görünür.

## Elevation & Depth

Derinlik ton katmanlarıyla verilir: zemin, onun üstünde beyazın saydam katmanları. Gölge yalnızca içeriğin üstünde yüzen, geçici ya da öne çıkan öğelerde vardır: komut paleti, menü, ipucu, yüzen çalar hapı ve kaydırıcı tutamacı. Gölge "bu geçici, içeriğin üstünde" anlamını taşır.

### Shadow Vocabulary
- **Yüzen** (`0 12px 32px rgba(0,0,0,0.55)`, tek katman; eskiden "iki katmanlı gölge şeffaf delik açıyor" sanılıyordu, 7 Kasım'da ölçüldü: delikler ekran görüntüsü betiğinin GDI+ yolundandı, ekranda yoktu, iki katman da doğru çiziliyor, bkz. vendor/gpui/OPENIFY.md madde 7; hap `0 12px 36px rgba(0,0,0,0.60)`: cam hap daha derin, yumuşak gölgeyle yüzer; kaydırıcı tutamacı ve bildirim şeridi `0 8px 28px rgba(0,0,0,0.55)`): Palet, menü, ipucu, başlık kapağı, kaydırıcı tutamacı, yüzen sağ panel.
- **Yerleşik** (`0 8px 24px rgba(0,0,0,0.40)`, `dock_shadow`; 7 Kasım, akşam): yerleşik "Şimdi çalıyor" paneli. Panel listenin yanında, aynı düzlemde; yüzen gölge onu geçici bir katman gibi gösterirdi. Sakin gölge yalnız paneli zeminden bir kat kaldırır, kenarı gece zemininde belli olur.

### Named Rules
**Yüzen Gölge Kuralı.** Kartlar, satırlar ve çubuklar gölgesizdir. Bir öğe gölge alıyorsa içeriğin üstünde yüzüyordur.

## Shapes

Yumuşak ama sıkı köşeler: 2 px ince çubuklar, 4 px küçük kapaklar ve kısayol etiketi, 6 px satırlar ve menü öğeleri, 8 px raf kapakları, "Tümü" karosu ve kütüphane arama kutusu, 12 px yüzen paneller (menü, ipucu), 20 px arama yüzeyi (Ctrl+K). Düğmeler yuvarlak (simge düğmesi, çal düğmesi) ya da hap biçimli (asıl düğme, süzgeç hapı, başlık çubuğundaki arama hapı, arama çipleri; `theme::R_FULL`); çalar hapı ve içindeki kapak da yuvarlak.

## Components

Bütün etkileşimli bileşenlerin beş durumu vardır ve bileşen vitrininde (`OPENIFY_GALLERY=1`) yan yana görülür. Klavye odağı kapak ışığı renginde bir halkadır. Zemini saydam öğelerde (simge düğmesi, menü öğesi) hep duran 2 px'lik kenar odakta renk alır; dolu zeminli düğmelerde (asıl düğme, çal düğmesi) halka düğmenin dışındadır, araya 2 px gece odası boşluğu girer (halka düğmenin üstüne çizilince renk farkı 1,6–1,9:1'de kalıyordu). Halka yalnızca klavyeyle (Tab) gezilirken görünür; fareyle tıklayınca odak bırakılır, böylece Enter / Boşluk tıklanan düğmeyi bir daha çalıştırmaz. **Klavye gezinmesi (8 Kasım):** Tab sırası görsel sırayı izler: başlık çubuğu (profil, geri, ileri, Ana sayfa / Keşfet / Kütüphane, arama, küçült / büyüt / kapat), sayfa (raf, listeyi çal, sayfanın kendi denetimleri: ayarlardaki seçenek düğmeleri, anahtarlar, aygıt alanı, düğmeler; Kütüphane süzgeçleri, sıralama, görünüm), sağ panel (sekmeler, tam ekran, kapat, kalp), hap (kalp, karıştır … ses). Enter / Boşluk odaktakini çalıştırır; seçenek gruplarında ← / → komşuya geçer; Esc odağı bırakır; kayan sayfada (ayarlar) odaklanan denetim görünür alana kaydırılır. Halka her yerde aynı ve hep klavye içindir: saydam öğede (simge düğmesi, sekme, profil, pencere düğmesi, kalp) hep duran 2 px'lik kenar odakta vurgu rengi alır; dolu öğede (anahtar, seçenek, süzgeç, asıl düğme) 2 px dışarıda, araya 2 px zemin boşluğu girer; alan gibi öğelerde (arama, aygıt alanı) kenar vurgu rengini alır. Fareyle tıklayınca halka kalkar. Menü ya da onay açan düğme klavyeyle tetiklenince odak köke döner (menü tuşları orada işlenir).

### Buttons
- **Shape:** Simge düğmesi 36 px yuvarlak, çal düğmesi 44 px yuvarlak, asıl düğme 40 px yüksek hap.
- **Primary:** Kapak ışığı zemin, gece odası yazı, 14 px yarı kalın, solunda 16 px simge ("Çal" / "Duraklat"). Küçük hâli (kayınca küçülen liste başlığında) 32 px yüksek, 13 px yazı, 14 px simge.
- **Hover / Pressed:** Asıl düğme üzerine gelince %16 açılır, basınca %14 koyulaşır. Simge düğmesi ton katmanı alır (%6 / %14). Beyaz çal düğmesi üzerine gelince kapak rengine %14 ısınır, basınca grileşir.
- **Disabled:** Simge soluk (%28), çal düğmesi zemini %24; tıklanmaz.
- **İpucu:** Sadece simgeden oluşan her düğmenin, ne yaptığını ve varsa kısayolunu söyleyen ipucu vardır (test ile korunur).
- **İpucu ve katman kuralı (12 Kasım):** Bir düğme menü, pencere ya da panel açıyorsa o yüzey açıkken düğmenin ipucu görünmez ve düğme "üzerine gelinmiş" zemini almaz. İki parça: (1) tam pencere katmanının (menü, onay / ad penceresi, eşitleme paneli, profil ve aygıt menüsü) başlık çubuğunun altını kaplayan görünmez bir engelleyici çocuğu vardır (`manage::layer_blocker`, `occlude`; eşitleme panelinde tekerlek geçsin diye `block_mouse_except_scroll`); katmanın kendisi engellemez, yoksa başlık çubuğundaki pencere düğmeleri ve sürükleme alanı ölür. Kapatma dinleyicileri (sol / sağ tık, tekerlek) engelleyicide de durur, çünkü o bölgede üst katmanınki çalışmaz. (2) Başlık çubuğundaki tetikleyici (profil hapı, eşitleme göstergesi) ipucunu yalnız kapalıyken kurar (`.when(!open, …)`). Ayrıca vendor `gpui`: tıklama bekleyen ipucunu da iptal eder (`vendor/gpui/OPENIFY.md` madde 9). Şarkı satırı menüsü beş grup halinde: sıra · beğen, radyo, albüm, sanatçı, katkılar · hiç çalma, gizle · bağlantı, Spotify'da aç · listeden çıkar; boş grup ayırıcı bırakmaz. Kart menüsünde hedef zaten ekrandaysa (açık liste, albüm / sanatçı sayfası) "Aç" yoktur.

### Inputs / Fields
- **Arama hapı (başlık çubuğu, 13 Kasım: "bir müzik uygulaması için sıkıcı"):** başlık çubuğunun ortasında, 36 px yüksek, **tam yuvarlak** (`R_FULL`); eni pencereye göre 520 (≥ 1500 px) / 480 (≥ 1180) / 420 (≥ 1000) / 280 (daha dar; `search_width_for`; hap hep pencere ortasında, sol yarıda profil hapı + gezinme + sekmeler için ≥ 240 px kalır, test). Dolgu **çalan kapağın ışığıyla tonlu, opak** (`theme::search_fill`: ışık rengi zeminle %50 karışır, parlaklık tavanı 0,040, üstüne %9 beyaz kat: kapak yokken de hap zeminden ayrılır; açık dolgu yüzeydeki giriş hapında). Solda 16 px büyüteç (`soft`), yer tutucu 14 px (Body) ve **vurgu rengine doğru ısıtılmış** (`search_hint`: ön plan rengi vurguya %22, %78 sağlam; her kapakta ≥ 4,5:1, test). Üzerine gelince beyaz %6 katman 140 ms'de belirir (`tween::hover_fill`). Klavye odağında vurgu renginde 1 px halka ve dışında soluk (%30) 2 px ikinci halka; **tuş rozeti ("Ctrl K") yok**. Tıklamak, odaktayken yazmaya başlamak (ilk harf girişe düşer) ve Ctrl+K aynı arama yüzeyini açar. Yer tutucu: "Bugün ne dinlemek istersin?" / "What are you in the mood for?".
- **Arama yüzeyi (Ctrl+K; 13 Kasım: "VS Code komut paleti gibi, gereksiz teknik"):** komut paleti değil, müzik arama yüzeyi. Ana pencerede arkadaki içeriği karartan perdenin (`SCRIM`, açılırken yumuşakça belirir) üstünde, başlık çubuğundaki hapla **aynı eksende ortalı**, üstü hapın üstünden (4 px) başlayan **tek cam yüzey**: 720 px (dar pencerede pencere − 32), en çok 700 px yükseklik (taşarsa kayar), köşe 20 px (`R_2XL`), zemin hapın camının (`PILL_GLASS_*`) kapak ışığına %60 çalan üst tonu (`surface_top`; etiketler her ışıkta ≥ 4,5:1, test), 1 px iç halka, derin yumuşak gölge (`capsule_shadow`). Üstte hapla aynı dilde **giriş hapı** (40 px, 18 px yazı, vurgu renginde ince halka, açık dolgu; imleç yazının sonunda). Her yerden açılan palet (Ctrl+Shift+Boşluk) aynı yüzeyi kendi penceresinde çizer.
  - **Kutu boşken** yalnız müzik, eylem yok: **"Son aramalar"** (çip: soluk `history` simgesi + yazı, 32 px, tam yuvarlak; tıklayınca yazıya döner; palette bir sonuca gidilince kaydedilir, ayarlarda `recent_searches`, en çok 8; yüzeyde en çok 6), **"Tekrar dinle"** (son çalınan en çok 6 liste / albüm / Beğenilenler: 84 px kapak, altında ad ve alt satır; üzerine gelince kapakta yuvarlak çal düğmesi) ve Keşfet'in tür ve ruh hâli karoları **bellekte (daha önce Keşfet açıldıysa) varsa** (en çok 8 renkli çip, türün renginde; yeni ağ isteği yok; çip tür sayfasını açar). Hiçbiri yoksa tek cümle: "Sanatçı, şarkı, albüm ve liste ara." Seçim halkası kutu boşken ilk ok / Tab tuşuna kadar görünmez (Enter yalnız görünür seçimi çalıştırır).
  - **Yazınca:** solda **"En iyi sonuç"** kartı (248 × 192 px: 88 px görsel — sanatçıda yuvarlak portre, albüm / liste / şarkıda kapak —, ad Figtree ExtraBold 18 px, tür etiketi "Sanatçı / Albüm / Şarkı / Liste" küçük hap; üzerine gelince ya da seçiliyken sağ altta 40 px vurgu renginde yuvarlak çal düğmesi: sanatçı bağlamını, albümü, listeyi, şarkıyı çalar; kartın kendisi açar / şarkıyı çalar), sağında **"Şarkılar"** (4 satır × 48 px: 40 px kapak — üzerine gelince çal işareti —, ad, sanatçı, süre). Altında **"Sanatçılar"** (yuvarlak 84 px portre, ad altında ortalı) ve **"Albümler ve listeler"** (kare 84 px kapak, ad ve alt satır; önce kütüphanedekiler, sonra Spotify) yatay sıraları (en çok 6); bölüm başlıkları sade, cümle biçiminde (büyük harfli üst başlık yok). **En iyi sonuç** adaylar (sanatçı, albüm / liste, şarkıların başı) arasında puan × tür ağırlığı (sanatçı 1,25, albüm 1,1, liste 1, şarkı 0,9) × eşleşme (ad aynen 2,4, ad başı 1,6) ile seçilir; seçilen kendi listesinden çıkar (iki kez görünmez). Kütüphane eşleşmeleri **anında**, Spotify cevabı 300 ms sonra; **cevap gelene dek eksik bölümlerin yerinde iskelet** (gerçek yerleşimle aynı ölçüde soluk kutular, 1,5 sn nabız; "Animasyonları azalt"ta sabit): "Searching Spotify…" yazısı yok. Spotify'a ulaşılamazsa kütüphane sonuçları durur, altta sakin tek satırlık not; hiçbir şey bulunamazsa "“…” için bir şey bulunamadı". En altta **sabit alt bağlantı** "“…” için tüm sonuçlar" (kaydırmaz; Ctrl+Enter yine çalışır).
  - **Eylemler** (sonraki / önceki şarkı, sessize al, Ana sayfa, Keşfet, Kütüphane, Yeni liste, Ayarlar, Radyo, Gizli oturum, Çıkış yap...) varsayılanda görünmez; yazılan bir eylemin adında (en az 2 harf) ya da açıklamasında (en az 4 harf) geçerse en altta tek bir **"Eylemler"** bölümünde, simgeli karoyla ve insan diliyle çıkar (en çok 3; tuş rozeti yok). Adı yazılınca (en az 3 harf, adın başı) seçili gelir: "sessize" yazıp Enter sessize alır; çıkış yap ikinci Enter ister.
  - **Klavye:** yazı girişe gider; ↑ ↓ ← → ızgarada gezer (en iyi sonuç kartından → ilk şarkıya, şarkıdan ← karta; sıralarda ← →; bölümler arasında ↑ ↓ en yakın sütuna, baştan sona sarar), Tab / Shift+Tab sıradaki / önceki (alt bağlantıya yalnız Tab ile gidilir), Enter açar ya da çalar, Esc ve Ctrl+K kapatır, Ctrl+Enter bütün sonuçlar, Ctrl+Backspace kelime siler. Seçili öğenin etrafında vurgu renginde 2 px halka (kartlarda, satırlarda, çiplerde, alt bağlantıda); seçili bölüm kendiliğinden görünür kaydırılır. **Hareket:** yüzey ve perde `motion()` (220 ms, ease-out quint) süresinde belirir (yüzey 8 px aşağı kayarak); "Animasyonları azalt"ta hiç sarmalanmaz. Kısayol listesi ("Kısayollar" eylemi) kodda durur (`SHORTCUTS`); orada tuş rozeti var.

### Covers
- **Beğenilenler:** sessiz ton karosu (`quiet-edge`) üstünde kapak ışığı renginde dolu kalp. Başka bir uygulamanın görselini taklit etmez, renk yalnızca kapaktan gelir.
- **Kapağı olmayan liste ya da şarkı:** aynı ton karosu, ortada etiket tonunda simge. Addan türetilen doygun degradeler kullanılmaz.

### Navigation
- **Raf kartı:** 136 px kapak (8 px köşe; 7 Kasım gecesi'ne dek 96 / 112), altında tek satır ad (Secondary, yarı kalın). Üzerine gelince kapakta %6 katman, basınca %14; seçili: kapak ışığı renginde 2 px halka (2 px boşlukla) ve ad; klavye odağı aynı halka. Çalan listenin kartında (yalnız çalarken) kapak köşesinde, koyu yuvarlak üstünde kapak ışığı renginde ses simgesi. Beğenilen şarkıların adı her yerde (raf, liste başlığı, palet, sayfalar, sıra) "Beğenilenler" (7 Kasım; önce yalnız kartta, 96 px'e sığsın diye). "Tümü" ızgarasındaki kart aynı.
- **Şerit kartı:** 40 px kapak (4 px köşe), ad yok; halka ve ses simgesi aynı (simge küçük).
- **"Tümü" karosu:** kapak boyunda, ton katmanı zemin (%6) ve ince kenar, ortada sayı (Headline, soluk), altında soluk "Tümü"; üzerine gelince %10, basınca %14. Şeritte "Tümü · N" yazılı 32 px düğme.
- **Gezinme:** raf ve şeritte ← → (Home / End) ile gezilir, Enter / Boşluk açar; Tab sırası arama → raf → listenin Çal düğmesi → hap (önceki, çal, sonraki, ilerleme, sessize al, ses). Tab ile rafa gelince imleç açık listede başlar.
- **Süzgeç hapı:** 32 px, yuvarlak; kapalıyken %6 ton katmanı ve soluk yazı (üzerine gelince %10, basınca %14), seçiliyken ön plan renginde dolu (çal düğmesi gibi üzerine gelince kapak rengine ısınır). Klavye odağı kapalıyken kenar, seçiliyken dış halka.
- **Şarkı satırı:** 56 px; üzerine gelince numara yerine çal simgesi, simge kendi başına bir düğmedir (tıklayınca çalar, satırın geri kalanı seçer); çalan satırda numara yerine ekolayzer ve kapak ışığı renginde ad. Sanatçı ve albüm adları tıklanabilir: tek satır yazı gibi kısalır, her ad ayrı (üzerine gelince el imleci ve 1 px alt çizgi, sayfası arkadan önceden istenir). Süreden önce 32 px'lik kalp sütunu: beğenilmişse hep görünür (kapak ışığı renginde dolu), değilse yalnız satırın üzerinde. Tek tık seçer, Ctrl+tık seçime ekler / çıkarır, Shift+tık aralık seçer, Ctrl+A hepsi; seçili satırlar %10 katman. Sağ tık menüyü açar (seçimin üstündeyse bütün seçim için). Kendi listende satır sürüklenir: bırakılacak yer önceden görünür (aşağı taşırken satırın altında, yukarı taşırken üstünde 2 px kapak ışığı çizgisi), imlecin yanında yüzen bir etiket (şarkı adı ya da "N şarkı").
- **Liste içinde bul (Ctrl+F):** liste başlığının altında 320 px'lik süzme kutusu (kenarı kapak ışığı), yanında "görünen / toplam"; yazdıkça satırlar süzülür (ad, sanatçı, albüm; Türkçe harfler sadeleşir, her kelime geçmeli). Esc kapatır.

### Kütüphane yönetimi (Adım 13)
- **Sağ tık menüsü:** menü paneli (240 px), satırda: Sıraya ekle, Sonra çal, Listeye ekle ›, ayırıcı, Beğen / Beğenilenlerden çıkar, Albüme git, Sanatçıya git (birden çok sanatçıda alt menü), Bağlantıyı kopyala, ayırıcı, kendi listende en altta tehlike renginde Listeden çıkar · Del. Tek şarkıda Bağlantıyı kopyala'nın altında Spotify'da aç. Kartta: Aç, Çal, Bağlantıyı kopyala, Spotify'da aç (şarkı, albüm, sanatçı, liste; geçici adresli liste ve Beğenilenler'de ikisi de yok); kendi listende Yeniden adlandır · F2 ve Listeyi sil; takip edilen listede Kütüphanenden kaldır; albümde Kütüphanene ekle / çıkar; sanatçıda ayırıcıdan sonra Takip et / Takipten çık (kartı bilinen yerlerden: kütüphane, açık sanatçı sayfası, benzer sanatçılar, arama sonuçları; bilinmiyorsa önerilmez, adsız kart eklenmez). "Bağlantıyı kopyala" `https://open.spotify.com/<tür>/<kimlik>` yazar ("Bağlantı kopyalandı" bildirimi); "Spotify'da aç" Spotify masaüstü uygulaması kuruluysa `spotify:` adresiyle (Windows kabuğu), değilse aynı yerin web bağlantısıyla tarayıcıda açar ("Spotify'da açılıyor…" / "Spotify uygulaması bulunamadı, tarayıcıda açılıyor…"). Alt menü öğenin sağında (sığmazsa solunda), en çok 360 px, kayar; "Listeye ekle" alt menüsünde en üstte "Yeni liste…". Dışarı tıklamak ya da tekerlek menüyü kapatır (arkadaki satır seçilmez). Klavye: ↑ ↓, → alt menü, ← geri, Enter, Esc; Shift+F10 seçili satırın menüsü.
- **Sanatçı takibi (11 Kasım):** sanatçı sayfasının eylem satırında Çal'ın yanında "Takip et" / "Takip ediliyor" (Çal ile aynı 40 px yükseklik ve yazı rolü, ikincil hap: ince kenar; `user-plus` / `user-check`; takip ediliyorken kenarı kapak ışığı renginde) ve "Diğer" (…) menüsü. Tıklayınca durum aynı karede döner (ekrandaki kütüphane bekleyen yazmayı da içerir); Spotify'a kalıcı kuyrukla (`edits.db`) arkadan yazılır, çevrimdışıyken sırada bekler, Ctrl+Z ve "Geri al" bildirimi ("X takip ediliyor" / "X takipten çıkıldı") çalışır, yazılamazsa eşitleme panelinde insan dilinde nedeniyle görünür ("Bırak" kartı eski hâline döndürür).
- **Geri al bildirimi:** sağ altta, hapın 12 px üstünde, 48 px yüksek yüzen panel: kapak ışığı renginde onay simgesi, ne olduğu, "Geri al · Ctrl Z". 6 sn sonra kalkar; işi bölmez. Sorun bildirimi kırmızı simgeyle.
- **Hap üstü şeritler (bildirim ve Connect "<aygıt> cihazında çalıyor", 12 Kasım):** hapın düz üst kenarının içinde durur: iki yanda 32 px (`S6`) içeride, alt kenarı hapın üst kenarından 12 px (`S3`) yukarıda; hapın yuvarlak uçlarının dışına taşmaz. Sorun bildiriminde yalnız simge kırmızı, yazı okunur kalır (ön plan tonu). Connect şeridinin simgesi `cast`, "Hiçbir yerde çalma" girdisininki `ban` (bir anlam, bir simge).
- **Eşitleme göstergesi:** başlık çubuğunda 32 px'lik hap: yazılıyorken soluk bulut simgesi + sayı, yazılamayan varsa tehlike renginde uyarı + sayı. Tıklayınca altında panel: bekleyen sayısı (çevrimdışıysa "bağlantı gelince yazılacak"), yazılamayan her değişiklik insan diliyle (ne ve neden; ham hata yok) ve "Yeniden dene" / "Bırak". Hiçbir şey beklemiyorsa gösterge yok.
- **Ad kutusu (yeni liste, yeniden adlandır):** perde üstünde ortada 400 px panel: Title başlık, 40 px giriş (kenarı kapak ışığı), Vazgeç (ikincil) ve Oluştur / Kaydet (küçük asıl düğme; ad boşken pasif). Enter onaylar, Esc vazgeçer.
- **Her işlem anında:** beğen, ekle, çıkar, taşı, adlandır, sil ekranda aynı karede uygulanır, Spotify'a arkadan yazılır; silme dahil hepsi geri alınabilir (Ctrl+Z).


### Menu
- **Panel:** Çubuk yüzeyi, 12 px köşe, yüzen gölge, 4 px iç boşluk, en az 200 px.
- **Öğe:** 32 px, solda 16 px simge sütunu, sağda soluk kısayol. Tehlikeli öğe tehlike yazısıyla, ayırıcıdan sonra en altta.

### Şimdi çalıyor (Adım 11 / 14)
- **Açılır düğme** (karıştır, tekrar, sıra, sözler): simge düğmesiyle aynı 36 px ve durumlar; açıkken simge kapak ışığı renginde, altında 4 px'lik nokta (renk farkı görülmese de durum belli). Tekrar tek şarkıdaysa simge "1"li. İpucu basınca ne olacağını söyler ("Karıştır" / "Karıştırmayı kapat", "Tekrarla" → "Bu şarkıyı tekrarla" → "Tekrarı kapat").
- **Sağ panel:** başlık çubuğunun altından (8 px) hapın üstüne (24 px) uzanan, sağda 16 px içeride, 360 px genişliğinde yüzen panel; çubuk yüzeyi, sessiz kenar, 12 px köşe, yüzen gölge (`float_shadow`, 0 12 32 %55; yerleşikken daha sakin "Yerleşik" gölge). (Eskiden gölgesizdi: "iki kenarda şeffaf delik" ekran görüntüsü betiğinin GDI+ yolundandı, ekranda yoktu; bütün betikler düz GDI bitmap'ine geçti, 7 Kasım; bkz. vendor/gpui/OPENIFY.md madde 7.) Hap ile panel arasında 24 px. Hap yatayda panelle çakışmayacak kadar genişse panel pencerenin altına (24 px) kadar iner. Sağdan 24 px kayıp belirerek gelir (220 ms). Başlığında "Şimdi çalıyor" / "Sıradaki" / "Sözler" süzgeç hapları (Şimdi çalıyor ilk ve varsayılan), tam ekran ve kapat düğmeleri. Esc yüzen paneli kapatır.
- **Yerleşik panel (7 Kasım; kullanıcı geri bildirimiyle 7 Kasım akşamı yeniden):** pencere 1720 px ve üstündeyse (32 kenar + en az 1240 liste + 32 boşluk + 400 panel + 16 sağ) panel listenin sağına yerleşir, pencerenin sağ kenarından 16 px içeride (yüzen panelle aynı yer; panelin içeriği böylece sağ kenardan 32 px içeride, sol sayfa kenarının aynası). İçerik (raf, liste, sayfalar, "Tümü") panelin solundaki bütün alanı kullanır; liste ile panel arasında listenin sağ sayfa kenarı (32 px), başka boşluk yok: hiçbir pencere eninde ölü boşluk kalmaz (eskiden artan yer iki dış kenara bölünüyordu, 1920'de panelin sağında 76 px boş). Panel pencerenin altına (24 px) kadar iner. Panel açılıp kapanınca içerik aynı karede yeni enine geçer, hap yerinden oynamaz (eskiden hap ve içerik 220 ms'de kayıyordu). Geri al bildirimi panelin solunda. Daha dar pencerede panel eskisi gibi yüzer ve kendiliğinden açılmaz. Açılışta açık mı olacağı Ayarlar'da ("Şimdi çalıyor panelini açık başlat", `now_panel`; panel çalışırken kapanınca tercih değişmez); Esc yerleşik paneli kapatmaz. Gösterecek hiçbir şey yokken (şarkı yok, son çalınan yok) panel açılmaz.
  - **Panel eni 400, sabit:** liste esnediği için panelin pencereyle büyümesine gerek yok; pencere boyutlanınca tek şey (liste) değişir. Gerekçe: kapak 366 px, 640 px'lik kaynak %150'de 549, %175'te 640 fiziksel px (hiç büyütülmez); paneldeki en uzun yazı (sanatçı tanıtımı, 13 px) 368 px'te ~57 harf, rahat okuma aralığının (45–75) ortası; ortadaki 800 px'lik hap 1680 px'ten geniş her pencerede panele 24 px'ten fazla yaklaşmaz. Eşikte liste 1240 px (şarkı ~576, albüm ~432), 1920'de 1440.
- **Hapın boş yeri:** düğmeler, kapak, kalp, sanatçı adları, ilerleme ve ses dışında hapa tıklamak paneli "Şimdi çalıyor" sekmesinde açar / kapatır (kapak yine tam ekran).
- **Şimdi çalıyor sekmesi:** büyük kapak (panelin iç eni: yüzen panelde 326 px, yerleşikte 366 px; **kısa panelde küçülür** (12 Kasım, `now_cover_side(panel_w, panel_h, has_next)`): ad ve sanatçı her zaman kaydırmadan görünür; sıradaki şarkı varsa "Sıradaki" başlığı ve bir tam satır da görünür, altında 12 px kalır (kapak = panel yüksekliği − 56 − 2 − 4 − 16 − ad bloğu − (24 + 20 + 8 + 56) − 12, `with_next`), bu kapağı 224 px'in altına indirecekse "Sıradaki" kaydırmaya bırakılır ve kapak sıradaki yokkenki boyunda (`− 16`) kalır; en az 160 px. 900×700'de (panel 527) 267 px, 1180×760'ta (588) 326 px değişmez, 1920'de değişmez; 8 px köşe; tıklayınca tam ekran; önce motorun 256 px'lik kapağı, tam ekranla ortak 640 px'lik kapak gelince üstünde kapak geçişiyle (150 ms, yavaşlayarak) belirir; sekme ya da panel kapanınca, pencere tepsiye gizlenince 640'lık bırakılır), altında 16 px ile ad (**Headline, Figtree 700, 28 px**, tek satır) ve 4 px altında tıklanabilir sanatçılar (**Title, 18 px**, ön plana yakın %86 ton; ad ile tek blok, kapakla aynı sol kenar), sağda kalp (ad satırının dikey ortasında). (7 Kasım gecesi: ad Title 18 px, sanatçı Body 14 px ikincil renkteydi; kullanıcı "şarkı adı ve sanatçı çok arka planda" dedi. Yeni boyla söz kesiti ~18 px kısaldı: 1920×1080'de üstte 5 yerine 4 önceki satır.) Sonra bölümler: aralarında 24 px, her biri bölüm başlığıyla (`panel-section-title`: Body 14 px, kalın, ön plan rengi; başlıkla içerik arası 8 px; 7 Kasım'a dek 12 px'lik soluk etiketti, küçük ve kopuk duruyordu; Title olsaydı altındaki söz satırlarıyla aynı boyda kalırdı): "Sıradaki" (56 px sıra satırı; tıklayınca Sıradaki sekmesi), "Sözler" (aşağıda; söz yoksa bölüm yok), "Sanatçı" (yalnız sayfası önbellekteyse: 48 px yuvarlak portre, ad, tanıtım yazısından en çok 220 harf; tıklayınca sanatçı sayfası). **Tek sol kenar:** sekme hapları, kapak, ad, sanatçı, bölüm başlıkları, satırların kapakları ve söz yazısı panelin 16 px kenar boşluğundan başlar; okunan söz satırının kapak ışığı çizgisi yazının 8 px solunda, kenar boşluğunun içinde durur (eskiden söz yazısı 12 px içerideydi). **Söz kesiti panelin kalan yüksekliğini doldurur** (7 Kasım; eskiden 3 satırdı, sıradaki ve sanatçı yokken panelin alt yarısı boş kalıyordu): okunan satır kesitin üçte birinde (sözler sekmesindeki odak yüksekliği; şarkının başında en üstte), önceki satırlar üstünde, sonrakiler altında; üst (önceki satır kaldıysa) ve alt kenarda panel yüzeyinden saydama 32 px'lik solma. Kaç satır kurulacağı panelin boyundan hesaplanır (ölçüm ya da fazladan kare yok), kesit yalnız satır değişince çizilir. Kısa panelde en az 152 px (okunan satır ve iki yanında solmanın dışında birer tam satır), gerisi kayar. Tıklayınca Sözler sekmesi; üzerine gelince sessiz satırlar açılır (sözler sekmesindeki gibi). Söz yoksa sütun içeriği kadar kalır, hiçbir şey esnetilmez (sakin). Hiçbir şey yüklü değilken: "Hiçbir şey çalmıyor" (Title) ve "Son çalınanlar" (en çok 8 liste, 56 px satır, 40 px kapak; üzerine gelince sağda çal simgesi, tıklayınca baştan çalar).
- **Sıra:** "Şimdi çalıyor", "Sıradaki" (sıraya eklenenler; sağında "Temizle"), "Sıradaki: <liste>" (karıştırma açıksa sağında "Karışık") bölümleri. Başlıklar satır boyunda, yazı alt kenarda (gruplar büyük boşlukla ayrılır). Satır 56 px: 40 px kapak (üzerine gelince ortasında çal), ad / sanatçı, süre; üzerine gelince sürenin yerinde "sıradan çıkar". Çalan satır kapak ışığının tonunda. Tek tık seçer, çift tık çalar. Satır sürüklenir: imleçte yüzen satır (yüzen gölge), bırakılacak yer kapak ışığı renginde 2 px'lik çizgi; listenin kenarına 48 px yaklaşınca liste kendiliğinden kayar. Değişiklik motorun cevabını beklemeden aynı karede görünür. Klavye: ↑ ↓ seç, Alt+↑ ↓ taşı, Del çıkar, Enter çal.
- **Sözler:** okunan satır tam ön plan renginde, solunda kapak ışığı renginde 2 px çizgi (panelde yazının 8 px solunda, kenar boşluğunda: yazı sekme haplarıyla aynı kenardan başlar; tam ekranda yazı 12 px içeride); diğerleri %64. Panelde Title (Figtree 700, 18 px), tam ekranda Headline (Figtree 800, 28 px). Okunan satır görünür alanın üçte birinde tutulur, değişince 300 ms'de kayar; tekerlekle kaydırınca 4 sn izleme durur. Satıra tıklamak oraya sarar. Eşzamansız sözde hepsi tam renk, üstte "Bu sözler şarkıyla eşzamanlı değil". Yoksa "Bu şarkının sözü yok", gelirken yerinde sessiz "Sözler getiriliyor…". Altta kaynak ("Sözler: Musixmatch"). **Kenar solması:** kaydırma alanının üst ve alt kenarında satır sert kesilmez. Panelde panel yüzeyinden (çubuk rengi) saydama giden 32 px'lik geçiş; gücü o kenarın ardında kalan kaydırılmış içerikle orantılı (en üstteyken ve içerik sığarken hiç yok), tıklamayı engellemez. Tam ekranda arka plan bulanık kapak olduğundan düz renkli geçiş yok: her satır, kenara 64 px kala saydamlaşmaya başlar ve kenara değince tümden saydam olur (kesilen satır görünmez, kenara gelen satır bütün olarak belirir).
- **Tam ekran:** pencereyi kaplar (Windows tam ekranı da). Kapak odadaki tek aydınlık nesne (12 px köşe, en çok 480 px); tam ekran açıkken çalan şarkının kapağı en büyük boyutuyla (640 px) ayrıca indirilir, motorun 256 px'lik saklanan kapağının üstüne kapak geçişiyle açılır; tam ekran kapanınca (panelin "Şimdi çalıyor" sekmesi de göstermiyorsa) bellekten bırakılır; arkasında aynı kapağın bulanık, kısık hâli (en parlak yeri göreli parlaklık 0,025, ışımanın tavanı: üstündeki sessiz yazı her kapakta ≥ 4,5:1, test). Solda kapak, altında ad (Headline) ve sanatçı (Title, %64); sağda sözler (en çok 640 px). 900 px'ten dar pencerede kapak ya da sözler (sözler düğmesi ikisi arasında gidip gelir). Altta denetimler (ilerleme; sözler, karıştır, önceki, çal, sonraki, tekrar, çık): 3 sn dokunulmayınca söner, fare ya da tuşla belirir (220 ms); üzerindeyken sönmez. F11 açar / kapatır, Esc çıkar.

### Ayar denetimleri (Adım 15)
- **Anahtar:** 44 × 24 hap, 20 px düğme (2 px boşluk). Kapalı: oluk rengi (`track-groove`) ve soluk düğme; açık: kapak ışığı rengi, düğme gece odası renginde (vurgunun üstünde koyu, çal düğmesi gibi). Üzerine gelince kapalıyken %10, açıkken vurgunun %16 açığı; klavye odağı dolu düğmelerdeki gibi dış halka. Düğme anında geçer (kare istemez).
- **Seçenek düğmesi (segmented):** siyah %32 zemin, beyaz %10 kenarlı hap; içinde 28 px'lik seçenekler (13 px). Seçili: ön plan renginde dolu, gece odası yazı, yarı kalın; seçili olmayan: soluk yazı, üzerine gelince %6 ton. Süzgeç hapıyla aynı dil.
- **Seçim alanı:** arama kutusuyla aynı (32 px, 8 px köşe); sağda aşağı ok. Açılan liste menü paneli (satır 32 px, en çok 8 satır, kayar), seçili satırda onay simgesi; dışarı tıklamak, tekerlek ya da Esc kapatır.
- **Onay kutusu** (çıkış yap, önbelleği temizle): perde üstünde ortada 400 px panel, ad kutusuyla aynı kalıp: Title başlık, Body açıklama (ne olacağını ve neyin etkilenmeyeceğini söyler), sağda Vazgeç (ikincil) ve eylem (küçük asıl düğme; çıkışta tehlike renginde). Enter onaylar, Esc ya da perdeye tıklamak vazgeçer.

### Tooltip
- Çubuk yüzeyi, 6 px köşe, yüzen gölge, 12 px yazı; kısayol varsa yanında kısayol etiketi.

## Logo

**İşaret: "çentikte nokta".** Kalın bir O halkası; sağ üstte, 45°'de halkanın kendi yolunda bir çentik açılır ve içinde yuvarlak bir ışık noktası durur. O "Openify"ın harfi, nokta odanın tek ışığı. Nokta halkanın dışında değil yolundadır: dışarıda ayrık duran nokta tek renkte ♂ işareti gibi okunuyordu (3. tur denemesi).

- **Uygulama simgesi** (`assets/logo/openify.svg`, `openify.ico`): gece odası karosu (224/256, köşe 56, `quiet-edge` → `night-room` dikey geçiş, üstte beyazın %16'sından %3'üne inen 2 px'lik kenar), içinde ön plan renginde işaret. Ölçüler 256'lık çizimde: halka dış yarıçap 76, kalınlık 32; nokta yarıçapı 21; çentik yarıçapı 34 (nokta ile halka arasında 13 boşluk). Simge koyu ve açık görev çubuğunda aynıdır; karonun kenarı açık zeminde de belli eder.
- **Tepsi işareti** (`assets/logo/tray-*.svg`, `tray-dark.ico`, `tray-light.ico`): karosuz, kalın hâli: halka dış 108, kalınlık 48; nokta 29; çentik 44. Uygulama işareti yalnız saydamlık olarak okur ve sistem temasının rengiyle boyar (koyu temada ön plan rengi, açıkta gece odası); tema değişince yeniden çizilir.
- **Tek Renk Kuralı.** Logo tek renktir: ön plan rengi (#F4F4F5) ya da gece odası (#0E0E10). Turuncu, degrade ya da kapaktan renk almaz; kapak ışığı arayüzündür, logonun değil.
- **Boşluk:** işaretin çevresinde en az noktanın çapı kadar boş alan; karoda bu karonun kendi iç boşluğudur, işaret karonun dışına taşmaz.
- **Boyutlar:** uygulama simgesi 16 / 20 / 24 / 32 / 40 / 48 / 64 / 256; tepsi 16 / 20 / 24 / 32 (%100–200). 16, 20 ve 24 px elle piksele oturtulmuş ayrı çizimler (`*-16.svg`, `*-20.svg`, `*-24.svg`): küçültülen ana çizimde çentik bulanıklaşıp nokta halkaya yapışıyordu. En küçük kullanım 16 px.
- **Gömülü yeri:** `build.rs` simgeyi exe'ye kaynak 1 olarak gömer (pencere, görev çubuğu, Başlat menüsü kısayolu); pencereye ölçeğine uygun büyük ve küçük simge ayrıca verilir (`src/system/icons.rs`); Velopack kurulumu aynı `.ico`'yu kullanır.
- **Yapma:** noktayı halkadan ayırıp dışarı taşıma, işarete renk ya da gölge verme, karoyu başka renge boyama, işareti döndürme.

## Do's and Don'ts

### Do:
- **Do** boşlukları, köşeleri ve yazı boyutlarını yalnızca `theme.rs`'teki ölçeklerden al; `design_rules` testi ölçek dışı değeri yakalar.
- **Do** aynı durumu her yerde aynı ton katmanıyla göster (üzerine gelme %6, seçili %10, basma %14).
- **Do** her yeni bileşeni beş durumuyla bileşen vitrinine ekle.
- **Do** kapaktan gelen rengi `ensure_contrast` ile okunur hâle getir.
- **Do** bir bloğu tek sol kenara diz; raf, liste başlığı ve satır içeriği aynı kenardan (`page_margin`: 32 / 48 / 64 px) başlar.

### Don't:
- **Don't** satıra, karta ya da çubuğa gölge verme; gölge yalnızca yüzen öğelerde.
- **Don't** başlığın üstüne büyük harfli tür etiketi koyma; tür bilgisi meta satırına yazılır.
- **Don't** ikincil yazıda sabit gri kullanma; ön plan renginin saydam tonunu kullan.
- **Don't** arayüze kapaktan bağımsız bir vurgu rengi ekleme.
- **Don't** Figtree'yi 18 px altında kullanma.

## Gezinme: Ana sayfa · Keşfet · Kütüphane (7 Kasım akşam)

- **Sekmeler:** başlık çubuğunda aramanın hemen solunda üç sekme (32 px, `R_LG` köşe, simge + ad). Seçili: %10 ton zemini ve ön plan rengi; seçili değil: soluk yazı, üzerine gelince 140 ms'de ton katmanı (`tween::hover_fill`). 1160 px'ten dar pencerede yalnız simge (ipucunda ad ve kısayol). Ctrl 1 / 2 / 3. Ana sayfa simge düğmesi kalktı (sekme var); başlık çubuğunda "Openify" yazısı yok (en solda profil hapı). Sanatçı, albüm, arama sayfalarında hiçbir sekme seçili değil. Geri / ileri sekmeler arasında da çalışır.
- **Raf yalnız Kütüphane sekmesinde** (açık liste ya da kütüphane görünümü); "Tümü" kartı yok, raf sağ kenara dek dolar ve Kütüphane görünümünde hep açıktır.
- **Kütüphane görünümü** (eski "Tümü"): rafın altında araç satırı: kütüphanede ara, Listeler / Albümler süzgeci (yeniden tıklayınca kalkar), "n / N", sıralama (dört seçenekli menü, aşağıdaki "Kütüphane düzeni"), ızgara / liste anahtarı, Yeni liste. Izgara ~168 px kartlarla (24 px aralar, rafı tekrarlamaz; klasörlü kütüphanede ağaç bozulmasın diye raf öğeleri ızgarada da durur; yukarıdaki "Tümü / Kütüphane görünümü"), liste 56 px satırlarla (40 px kapak, ad, açıklama, sağda tür). Seçimler `settings.json`'da. **Sanatçılar (11 Kasım):** üçüncü süzgeç hapı "Sanatçılar" takip edilen sanatçıları yuvarlak portre kartlarıyla (ızgarada kapak boyunda, listede 40 px yuvarlak kapak; altyazı "Sanatçı", sağda yinelenmez) gösterir; tıklayınca sanatçı sayfası açılır, sağ tık sanatçı menüsü. Sanatçılar süzgeçsiz "Tümü"de ve rafta görünmez (yüzlerce sanatçı listeleri boğmasın); süzgeç kapalıyken yazı yazılınca aramaya katılır (altyazıları aramaya girmez, "ar" yazınca hepsi gelmesin). Sıra: "Son takip edilen" (en son takip edilen önde; sanatçı için "son çalınan" verisi yok, süzgeçte sıralama düğmesi bu adı alır) ya da Ad. Hiç takip yoksa sakin boş durum ("Henüz sanatçı yok"). Eklenme sıralaması artık var (aşağıdaki "Kütüphane düzeni").
- **Ana sayfa:** yukarıdaki "Açılış ve ana sayfa" (7 Kasım gecesi): selamlama, kahraman satırı (Kaldığın yerden + günün önerisi), Çok dinlediklerin, Senin için yeni, Listelerin. Akış gelene dek "Senin için yeni" yerinde iskelet; gelmezse kütüphanedeki Spotify listeleri.
- **Keşfet:** Yeni çıkanlar, Trend listeler, Tür ve ruh hâli (96 px karolar, türün rengi %22 koyulmuş; beyaz yazı ≥ 4,5:1, testli), Benzer sanatçılardan öneriler (önbellekteki listelerden, kütüphanede olmayanlar). Tür sayfası: pencere ışıması türün renginde, altında bölüm bölüm kartlar. Yükleniyor: iskelet satırlar; hepsi başarısızsa sayfa ortasında durum görünümü ve "Yeniden dene".
- **Arama çipleri (11 Kasım):** tam arama sayfasında başlığın altında tek sıra: Hepsi · Şarkılar · Sanatçılar · Albümler · Listeler · Podcast'ler · Profiller (`chip`, 32 px; seçili dolu, `src/ui/search_page.rs`). Hepsi = karışık görünüm (her tür bir sıra + "Tümünü göster", şarkıda ilk 5; podcast bölümleri yok); bir türün çipi yalnız o türün bütün sonuçlarını başlıksız açar (Podcast'ler çipi podcast'leri ve Bölümler'i iki başlıkla). Çipler Tab durağıdır (`seg:search:<tür>`, `tab::PAGE`, çizim sırasıyla), ← / → komşu çipe geçer, Enter / Boşluk seçer. Yeni aramada "Hepsi"ne döner; hiç sonuç yoksa çip sırası görünmez, seçili türde sonuç yoksa altında sessiz tek satır. Podcast, bölüm ve profil için uygulamada sayfa yok: kartlar sanatçı / albüm kartlarıyla aynı ölçüde (podcast ve bölüm kare, profil yuvarlak), tıklayınca `open.spotify.com` sayfası varsayılan tarayıcıda açılır (ipucu "Spotify'da aç"). Sonuçlar mevcut `searchDesktop` sorgusunun `podcasts`, `episodes`, `users` alanlarından çözülür (sorgu ve hash değişmedi); kayıtlı biçim `src/engine/fixtures/search_desktop.json`.
- **Son çalınanlar (11 Kasım):** `Route::Recent`, Ana sayfa sekmesi seçili kalır. Başlık + "n liste ve albüm", altında kütüphanedeki son çalınan liste / albüm / Beğenilenler kartları (en yenisi başta, `Settings::recent`, en çok 50). Boşken sayfa ortasında durum görünümü ("Henüz bir şey çalmadın", tek eylem "Keşfet'e bak"). Ana sayfada "Çok dinlediklerin" rafının sağında "Son çalınanlar" çipi (kayıt boş değilken) ve palette "Son çalınanlar" komutu açar. Tek şarkı çalmalar ve kütüphanede olmayan bağlamlar (kaydedilmemiş albüm, silinmiş liste) kayıtta yok / gösterilmez.

## Karşılama ve "veri yok" durumları (11 Kasım)

Önbellekte kütüphane yokken (ilk açılış, önbellek silindi) bağlanılamıyorsa ya da giriş gerekiyorsa Ana sayfa, Keşfet ve Kütüphane sekmesi sonsuza dek gri iskelet gösteriyordu; tek ipucu hapın küçük durum satırıydı ("Çevrimdışı · Bağlantı bekleniyor", "Giriş gerekli"). Artık o sayfalarda iskeletin yerine tam sayfa durum var (`src/ui/welcome.rs`, bileşen `components::full_state`). Önbellekte veri varsa bugünkü davranış sürer: içerik gösterilir, durum yalnız hapta.

- **Yerleşim:** sayfanın (hapın üstünde kalan alanın) ortasında tek sütun, en çok 440 px: işaret (64 px), Headline başlık, Body açıklama (soluk, ortalı), 24 px sonra eylemler (asıl düğme 40 px, yanlarında 12 px aralıkla ikincil / sessiz düğme), 16 px sonra tek satır not (etiket tonu). Etiket üstü (kicker) yok. İşaret: karşılama ve giriş bekleyişinde uygulama işareti (`assets/icons/openify-mark.svg`, karosuz "çentikte nokta", ön plan rengi); diğer durumlarda 64 px yer tutucu karoda (12 px köşe) soluk 28 px simge.
- **Durumlar** (bağlantı durumundan, `Connection`; `Shell::gate_for`): **giriş gerekli** (karşılama: "Openify'a hoş geldin", tek satır "Spotify için sakin ve hızlı bir masaüstü çalar.", asıl düğme "Spotify ile giriş yap" = `Command::StartLogin`, not "Müzik çalmak için Spotify Premium gerekir."); **giriş bekleniyor** (başlık "Girişi tamamla", "Tarayıcında erişime izin ver. Bu sayfa kendiliğinden devam eder.", yalnız "Girişi iptal et" ikincil düğmesi; hareketli gösterge yok, 3 dakikalık bekleyiş için sürekli kare istemek yerine sakin yazı); **giriş yapılamadı** (neden `msg::login_failed_text`, "Yeniden dene"); **çevrimdışı** ("Çevrimdışısın", kendiliğinden yeniden denendiği söylenir, "Şimdi dene" = `RefreshLibrary`); **Premium gerekli** (açıklama, asıl düğme "Premium'a geç" = spotify.com/premium kabukta açılır (sınama kopyalarında hiç açılmaz), "Yeniden kontrol et" = `RefreshLibrary`, sessiz "Çıkış yap"); **ses motoru durdu** ("Yeniden başlat"); **uzun sürüyor** (bağlanma ya da yükleme 15 sn'yi aştı: "Her zamankinden uzun sürüyor", "Şimdi dene"); kütüphane başka bir nedenle gelmediyse `explain_error` başlığı + "Yeniden dene". Keşfet'te aynı durumlar yalnız hiçbir bölümün verisi yokken (önbellekte Keşfet verisi varsa içerik).
- **"Şimdi dene" geri bildirimi:** basınca durum kalır, düğme pasif ve "Deneniyor…" olur (en çok 6 sn ya da bir sonuç gelene dek); iskeletle yer değiştirip geri gelmez. Yeniden deneme yükleme saatini sıfırlar.
- **İskelet yalnız gerçekten yüklenirken:** bağlantı sonucu çoğu kez ilk saniyeden önce gelir; açılışta ilk 400 ms ve her sayfanın ilk 120 ms'sinde iskelet çizilmez (önbellekteki veri birkaç karede gelir, iskelet çakıp silinmesin), sonra iskelet, 15 sn'de "uzun sürüyor" durumu. Ana sayfada "Senin için yeni" iskeleti de yalnız veri gelebilirken (bağlanıyor ya da bağlı, yavaş değil) çizilir; yoksa kütüphanedeki Spotify listeleri. Kütüphane sekmesinde durum varken raf çizilmez (altında gri kartlar durmasın).
- **İskelet gerçek sayfanın kutusudur:** iskelet, sayfanın içerikle aynı yerde başlayan hâlidir; olmayan parça çizilmez. Kapaksız sayfalarda (tür sayfası, Son çalınanlar) kapak karesi ve Çal hapı yok: başlık çubuğu `page_header(None, ..)`daki başlıkla aynı x/y'de (üst 48 px, 280 × Display satır yüksekliği, 8 px aralık, 180 × 10 meta çubuğu; `pages::page_skeleton_plain`), tür sayfasında kartlardan önce 48 px'lik bölüm başlığı satırı (140 × 18 çubuk). İçerik gelince hiçbir şey zıplamaz; ölçüler `plain_header_geometry`den gelir ve testlidir.
- **Geçiş:** bir durum belirirken ve iskelet ya da durumdan içeriğe geçilirken yeni görünen parça %30 saydamlıktan ve 8 px aşağıdan 220 ms'de (`motion`, quint) gelir (panel sekmeleriyle aynı %30 taban: arada boş kare yok); içerik sarmalı 500 ms sonra bırakılır; "animasyonları azalt"ta sarmal yok. Yalnız iskelet ya da durum görüldükten sonra: önbellekten ilk karede gelen içerik eskisi gibi (sayfa geçişiyle) açılır.
- **Klavye:** asıl düğme (`gate-primary`) bir Tab durağıdır (`tab::PAGE`); sayfada durum varken ilk Tab (odak çapadayken) başlık çubuğunun on durağından geçmeden doğrudan ona iner, Enter / Boşluk çalıştırır. Durum değişince (Giriş yap → Girişi iptal et) aynı tutamaç kaldığı için odak yerinde kalır. İkincil düğmeler (Yeniden kontrol et, Çıkış yap) sıradaki duraklar.
- **Yeniden istek:** motor bağlantı yokken sorulan Ana sayfa / Keşfet verisini çevrimdışı olunca atıyordu (sayfa bağlantı gelse de veriyi hiç almıyordu); `Event::Connected`'da açık sayfanın eksik verisi yeniden istenir (`Shell::reask_explore`).
- **Sınama kancaları:** `OPENIFY_TEST_LINK=needs_login|needs_premium|stall` (`stall`: bağlanma hiç bitmez), `OPENIFY_TEST_OFFLINE=1`, `OPENIFY_TEST_PANIC_ENGINE=1` (motor 2 sn sonra çöker), `OPENIFY_TEST_LOGIN_MS=<ms>` (sahte giriş bekleyişi; dolunca "giriş yapılamadı"), `OPENIFY_TEST_GATE_MS="iskelet,uzun"` (ms; 400 / 15000 yerine). Hiçbiri tarayıcı açmaz.

## Ayarlar ve profil menüsü (Adım 15, 7 Kasım gece)

Discord uygulama kimliği yapılandırılmışsa Ayarlar'da **Entegrasyonlar** bölümü
görünür: **Dinlediklerimi Discord'da paylaş** anahtarı, varsayılan kapalı.
Mevcut `settings_section`, `setting_row` ve anahtar desenini kullanır; yeni
renk, boyut veya boşluk yoktur. Açıklama, şarkı ve ilerlemenin paylaşılacağını,
duraklatmada ve gizli oturumda gizleneceğini söyler. Duraklatmada etkinlik
temizlenir: ilerlemeye devam eden yanıltıcı bir Discord sayacı bırakılmaz.
Kimlik boşken bölüm ve satır tamamen gizlidir. Tüm metinler İngilizce ve Türkçe
i18n tablosundadır.

- **Giriş:** başlık çubuğunun **en solunda** (eski "Openify" yazısının yerinde; kullanıcı sağdaki küçük yuvarlak düğmeyi bulamadı) 32 px'lik profil hapı: 24 px yuvarlak avatar (hesabın baş harfi; ad yoksa ya da Spotify'ın ürettiği uzun bir kimlikse kişi simgesi) + hesap adı + aşağı ok (`chevron-down`). Zemin %6 ton katmanı, üzerine gelince / basınca 140 ms'de koyulaşır (`tween::hover_fill`), menü açıkken %10; klavye odağında 2 px vurgu halkası. Ad en çok 140 px, sığmazsa "…" (ipucunda tamamı). Sol yarıdaki yer (pencere − arama) / 2 eksi gezinme düğmeleri ve sekmeler kadar; ad için 48 px bile kalmıyorsa (≈ 1600 px altı, ad uzunluğuna göre) hap yalnız avatar + ok olur. Hap sürükleme alanının dışında (tıklanır); boş alanlar pencereyi sürükler. "Openify" yazısı başlık çubuğundan kalktı (uygulama adı pencere başlığında ve görev çubuğunda). **Spotify profil fotoğrafı yok:** oturum yalnız kullanıcı adını veriyor; fotoğraf için ayrı bir Spotify isteği gerekir, ayarlar için istek eklenmedi (ileride `user-profile-view` sorgusuyla eklenebilir). Tıklayınca hapın altında, sol kenarına hizalı 272 px'lik menü paneli: üstte hesap bloğu (36 px yuvarlak baş harf, ad, "Spotify hesabı"), ayırıcı, **Ayarlar** (Ctrl ,), **Hakkında** (Ayarlar'ı en altına kaydırarak açar), ayırıcı, tehlike renginde **Çıkış yap** (uygulama içi onay kutusu). Klavye: ↑ ↓, Enter, Esc; dışarı tıklamak ya da tekerlek kapatır. **Ctrl ,** Ayarlar'ı doğrudan açar ve palette "Ayarlar" komutu vardır.
- **Sayfa:** geçmişte bir yer (`Route::Settings`; geri / ileri, Alt+← çalışır), hiçbir sekme seçili değil. Sayfa başlığı (Display, "Ayarlar", altında "Openify'ı kendine göre ayarla"), altında tek sütun: en çok 720 px, `page_margin` kenarından, sola hizalı (diğer sayfalarla aynı sol kenar). Bölüm başlığı (Title, Figtree 800, 48 px alan) ve satırlar; satırlar kutusuz, aralarında %6 çizgi, her biri ad (Body, yarı kalın) + tek cümle açıklama (Secondary, soluk; TEK satır, sığmazsa "…" ve ipucunda tamamı: denetimin altına ya da yanına taşmaz) ve sağda denetim. Sorun notu (ör. kısayol alınamadı) satırın altında tehlike renginde.
- **Bölümler ve satırlar** (düz Türkçe, teknik terim yok):
  - **Ses:** Ses kalitesi (Normal / Yüksek / Çok yüksek = 96 / 160 / 320 kbps, sıradaki şarkıdan geçerli), Ses düzeylerini eşitle (anahtar), Çıkış aygıtı (liste; "Sistem varsayılanı" ya da bir aygıt, çalarken kesintisiz geçer).
  - **Görünüm ve davranış:** Animasyonları azalt (Sistem / Açık / Kapalı; Sistem Windows'un ayarını izler), Şimdi çalıyor panelini açık başlat, Kapat düğmesi (Tepsiye gizle / Uygulamadan çık), Her yerden arama (Ctrl+Shift+Boşluk).
  - **Başlangıç:** Windows açılınca başlat (tepside gizli; yalnız kullanıcı anahtarı çevirince yazılır), Kaldığın yerden devam et.
  - **Depolama ve hesap:** Önbellek (boyut, "Önbelleği temizle" + onay), Önbellek konumu (yol, "Klasörü aç"), Spotify hesabı (ad, "Çıkış yap" + onay).
  - **Hakkında:** sürüm ve güncelleme durumu ("Şimdi denetle"), Lisanslar (açık kaynak bileşenler, yazı tipi / simge lisans metinleri tıklayınca), "Spotify ile bağlantılı değildir" notu.
- **Şarkılar arası geçiş:** yalnız boşluksuz (gapless) var, ayar satırı yok. Yumuşak geçiş (crossfade) kendi çıkışımızda çalar tek akışı yazdığı için (sıradaki şarkı çalara ancak öncekinin sonunda gelir) iki şarkıyı üst üste bindiremiyor: gerçek bir yumuşak geçiş çaların iki şarkıyı birden çözmesini gerektirir. Boş bir "yakında" satırı yok.
- **Yapılmayan:** önbellek konumunu değiştirme (gösterilir, `Klasörü aç` ile açılır; değiştirmek ses önbelleğini, kapakları ve SQLite'ı taşımayı gerektirir).
- **Davranış notları:** her değişiklik hemen uygulanır ve `settings.json`'a yazılır. "Şimdi çalıyor panelini açık başlat" bir tercihtir: panel çalışırken açılıp kapanınca değişmez (eskiden son kapanış hatırlanıyordu). Çıkış yap kayıtlı girişi siler ve Openify'ı kapatır; palette "Çıkış yap" aynı işi paletin kendi onayıyla yapar. Önbellek temizlenince şarkı dosyaları ve kapaklar silinir; kütüphane veritabanı (bekleyen yazmalar, kaldığı yer, sözler) ve giriş silinmez.

## Ekolayzer (Ayarlar → Ses'in altında, 11 Kasım)

- **Yer:** ayrı sayfa değil, Ayarlar'da "Ses"in hemen altında "Ekolayzer" bölümü (Başlık rolü, diğer bölümlerle aynı sütun, en çok 720 px). Eyebrow / küçük üst başlık yok.
- **Satırlar (üstten alta):** (1) "Ekolayzeri kullan" anahtarı + tek cümle; (2) sürgüler; (3) hazır ayarlar + "Sıfırla"; (4) "Kulaklık profili" satırı (arama kutusu) ve altında sonuçlar, seçili profil kartı, OPRA kaynak notu.
- **Sürgüler:** 10 dikey sürgü (31,5 Hz … 16 kHz), solda ince bir çizgiyle ayrılmış ön kazanç sürgüsü. Oluk 148 px (2×12 dB, 0,5 dB adım), ortada sıfır çizgisi, sıfırdan değere vurgu renginde dolgu, 14 px beyaz tutamaç (odakta vurgu halkası); üstte değer ("+3,5", "−2", "0"; eksi işareti gerçek eksi, Türkçede ondalık virgül), altta frekans etiketi (12 px). Fareyle tıklayınca değere atlar, sürüklenir; çift tık sıfırlar. Klavyede her sürgü Tab durağı: ↑ ↓ 0,5 dB, PgUp / PgDn 3 dB, ← → komşu sürgü, Delete / Backspace / 0 sıfırlar. Kapalıyken sürgüler soluk (%62) ama kullanılır: bir sürgüyü, hazır ayarı ya da profili değiştirmek ekolayzeri AÇAR (duyulmayan bir ayar bırakılmaz).
- **Anında duyulur:** her değişiklikte `Command::SetEq` gider (sürüklerken her fare hareketinde; motor katsayıyı 8 blokta süzer, tıkırtı yok); diske yalnız sürükleme bitince / tuşta yazılır.
- **Hazır ayarlar** (segmentli denetim, yalnız beş): Düz, Bas artır, Tiz artır, Vokal, Akustik; kazançlar bir hazır ayara tam eşitse o seçili görünür, elle değişince ya da profil seçilince hiçbiri seçili değildir. "Sıfırla": bütün bantlar ve ön kazanç 0, profil kalkar (açık / kapalı durumu değişmez).
- **Kulaklık profili (OPRA, CC BY-SA 4.0):** arama kutusuna (Tab durağı; yazı `eq_key`'te) ilk dokunuşta ya da yazışta veri önbellekten gelir / indirilir; yüklenirken "Kulaklık profilleri alınıyor…", indirilemezse (ve önbellek yoksa) tek satır "Kulaklık profilleri alınamadı." + "Yeniden dene". Sonuçlar (en çok 6 satır) koyu girdi kutusunda, satır başına model adı (Body, orta) ve altında "yaratıcı · ayrıntı"; ↑ ↓ + Enter ya da tıklama seçer. Seçilince profil 10 banda uydurulur, ekolayzer açılır ve profil kartı çıkar: ad, "Ölçüm: <yaratıcı> · OPRA (CC BY-SA 4.0)" (OPRA kısmı GitHub sayfasına bağlantı), ayrıntı satırı, "10 banda yaklaştırıldı, ortalama sapma 0,8 dB", "Kaldır". Altta her zaman kısa OPRA açıklaması. Bir sürgü elle oynatılınca profil kartı kalkar (artık o profil değil).
- **Kaynak gösterimi:** profil seçiliyken atıf satırı; Hakkında → Lisanslar'da "OPRA · CC BY-SA 4.0" satırı (tıklayınca kısa bildirim metni). OPRA logosu gömülmedi.
- **Bellek:** OPRA kataloğu (12,6 MB metinden ~10 MB) yalnız Ayarlar açıkken ve arama kullanıldıysa bellektedir; sayfadan çıkınca atılır.

## Dil ve metin (i18n, 11 Kasım)

Arayüz İngilizce + Türkçe. Dil: açılışta Windows'un görüntü dili (Türkçe ise `tr`, değilse `en`); Ayarlar → "Görünüm ve davranış" → **Dil / Language** (Sistem · English · Türkçe) ile değişir, `settings.json`'da `language` (`system|en|tr`, `serde(default)`). Seçim HEMEN uygulanır: `Shell::apply_prefs` → `i18n::apply_preference`, ardından kök `notify` (bütün görünümler `Shell`'i izlediği için hepsi yenilenir; yeniden başlatma gerekmez).

- **Yazılar tek yerde:** `src/i18n/table.rs`. Her girdi `Anahtar { en: "…", tr: "…" }`; iki dilden biri eksikse DERLEME HATASI (makro iki alanı da ister), anahtar adı yanlışsa derleme hatası (anahtarlar bir `enum`). Kodda ham görünen yazı olmaz: `t!(Anahtar)` (`&'static str`), `tf!(Anahtar, a, b)` (`{}` / `{0}` yer tutuculu `String`), `tp!(Anahtar, n[, ekstra…])` (çoğul: `"{} song|{} songs"`, sayı binlik ayraçlı: 1,234 / 1.234), `k!(Anahtar)` (sabit tabloda anahtar saklamak için; çözmek `i18n::t(k)`). Sabit (`const`) tablolara `t!` konmaz: işlev ya da anahtar tablosu kullan.
- **Yeni yazı eklemek:** (1) `table.rs`'in sonuna anahtar adı (ekran + anlam: `SettingsRowQuality`) + `en` + `tr`; (2) kodda `use crate::t;` (ya da `tf`, `tp`) ve `t!(…)`; (3) `cargo test` (tablo, yer tutucu ve çoğul testleri + kaynak taraması).
- **Üslup (İngilizce):** kısa, doğal, sakin; Apple Music / Spotify tonu; birebir çeviri değil ("Make Openify your own", "Nothing in the queue"). Cümle büyük harfle, başlıklar da cümle düzeninde ("Liked Songs" Spotify'ın özel adı), noktalama Türkçedeki gibi (tek cümlede nokta yok, çok cümlede var). Türkçe metinler eskisi gibi kalır (yalnız açıkça bozuksa düzeltilir).
- **Uzunluk:** İngilizce yazılar çoğu yerde Türkçeden kısa ya da eşit; en dar yerler (başlık sekmeleri, hap, Ayarlar satır açıklamaları, ana sayfa kahramanı, boş durumlar) 1180×760'ta Türkçe ile aynı kapta sığmalı (`OPENIFY_TEST_LANG=en`/`tr` ile iki dilde ekran sına).
- **Sayı, süre, harf:** `i18n::num` (binlik ayraç), `localize_decimal` ("1,2 MB" / "1.2 MB"), süre `"1 sa 12 dk"` / `"1 hr 12 min"` (`DurHoursMinutes`), büyük/küçük harf `i18n::upper` / `lower` (Türkçede İ/ı doğru; `to_lowercase()` "İ"yi bozar). Selamlama günün saatine göre (`HomeGreet*`).
- **Uygulama adı TEK sabit:** `src/brand.rs` (`app_name!()`, `app_name_acc!()` = Türkçe belirtme hâli "Openify'ı", ek ünlü uyumuna göre elle yazılır; `APP_NAME` pencere başlığı, tepsi ipucu, medya başlığı için). Adı değiştirmek görünen yazı için bu iki makroyu düzenlemek. KİMLİK olanlar (AUMID, kayıt anahtarı, klasörler, tek kopya kilidi…) görünür ad değildir, `brand.rs` başındaki listede: ayrı karar.
- **Kalıcı veride dil yok:** kütüphane altyazıları (`"29 şarkı"`, `"Takip edilen · 29 şarkı"`, `"Spotify · 50 şarkı"`, `"Sanatçı"`, `"Liste"`) SQLite'ta kanonik (Türkçe) biçimde durur, ekranda `i18n::canon::localize` ile çevrilir (bkz. `src/i18n/canon.rs`); böylece dil değişince eski dilde kalmaz. Altyazıyı ÜRETEN ve AYRIŞTIRAN kod yalnız `canon`'dadır.
- **Türkçe kalanlar (bilerek):** günlük satırları (`log::`), `anyhow` / `bail!` hata zincirleri ve motor tanı nedenleri (ekranda `explain_error` / `explain_edit_failure` sınıflayıp `t!` yazısı verir), terminal komutunun (`openify <bağlantı>`) hata iletileri, geliştirme vitrini (`OPENIFY_GALLERY`) ve sahte çalma sözü. Bunlar `src/i18n/tests.rs` içindeki `ALLOW` listesinde gerekçesiyle durur.
- **Regresyon testi:** `i18n::tests::no_raw_turkish_text_in_sources` (kaynakta, günlük / hata bağlamı dışında Türkçe harfli ham yazı), `moved_turkish_strings_do_not_reappear_as_literals` (tablodaki Türkçe yazı kodda ham geri gelmesin; "Ana sayfa" gibi Türkçe harfsiz olanları da yakalar), tablo bütünlüğü (iki dil dolu, yer tutucu / çoğul tutarlı, İngilizcede Türkçe harf yok).

## Karıştırma ve sıra kuralları, kısayol yardımı (11 Kasım)

- **Ayarlar → "Karıştırma ve sıra"** (Ses'in altında): "Her şarkının tek sürümü" (varsayılan açık; yalnız karıştırılmış bağlamda), "Açık sözlü şarkıları atla" (varsayılan kapalı), "Hiç çalma" listesi (sanatçı satırı + "Yeniden izin ver"; boşken tek cümle yönlendirme). En altta tek satır not: aynı sanatçı karıştırmada birbirinden ayrı tutulur (ayar değil, hep açık). Satırlar mevcut `setting_row` / anahtar düzeniyle; hepsi klavye durağı.
- **"Bu sanatçıyı hiç çalma"**: şarkı satırı menüsü (sanatçı adlarından; birden çoksa alt menü), sanatçı kartı menüsü (adı bilinen yerlerden) ve çalan şarkının sanatçı adlarına sağ tık (hap, panel). Kara listedekine "{} sanatçısına yeniden izin ver" gelir. Bildirim kısa ve geri alma düğmesiz (Ayarlar'da listeden çıkarılır).
- **Eşitleme düzeyi**: Ayarlar → Ses'teki anahtar, Kapalı / Sessiz / Normal / Yüksek seçeneğine döndü (−23 / −14 / −11 LUFS; Normal'e göre −9 / 0 / +3 dB ön kazanç).
- **Kısayol yardımı**: `?` ya da Ctrl+/ ya da palette "Kısayollar". Pencereyi karartan katman (onay kutusunun karartması ve gölgesiyle aynı dil), 880 px'e kadar iki sütun (1000 px'in altında tek sütun, 520 px), başlık + Kapat düğmesi, gruplar: Çalma, Paneller ve pencere, Ara ve git, Listeler, Tuşlarla gezinme. Satır: solda ne yaptığı (T_SECONDARY, soluk), sağda her tuş ayrı etiket (`kbd`). Esc ya da `?` kapatır, odak açıldığı yere döner; ↑ ↓ / Page Up / Page Down / Home / End kaydırır; tam ekranda açılmaz (katmanlar çizilmez).


## Arkadaş etkinliği (12 Kasım)

Spotify masaüstündeki "Friend Activity": arkadaşların son dinlediği şarkı. İsteğe bağlı ve varsayılan KAPALI (Ayarlar → Görünüm ve davranış → "Arkadaş etkinliğini göster": "Arkadaşlarının son dinlediği şarkıyı gösterir. Yalnız dinlediklerini paylaşan arkadaşlar görünür."); kapalıyken ne düğme vardır ne de Spotify'a tek istek gider. Gayriresmî bir uç kullanılır, canlı yanıtla henüz doğrulanmadı (`src/engine/friends.rs`, `src/ui/friends.rs`).

- **Yer (karar, 12 Kasım "kaplı liste" v2):** "Şimdi çalıyor / Sıradaki / Sözler" panelinin AYNI sağ yuvasında dördüncü bir panel türü (`Panel::Friends`); aynı yuvayı paylaştıkları için biri açılınca öbürü kapanır. **Yerleşik, örtmez:** içerik alanı en dar pencere eninin (760) altına inmedikçe panel sayfanın yanına yerleşir ve içeriği daraltır: eşik 760 + 360 + sağ boşluk 16 = **1136 px** (`now_panel::FRIENDS_DOCK_MIN_WIDTH`; 1180×760 ve 1920×1080 yerleşik). Şimdi çalıyor paneli kendi eşiğinde (1720, geniş liste korunur) kalır; `DockKind::{NOW, FRIENDS}` iki kuralı ayırır. 900×700'de yerleşemez, içeriğin üstünde yüzer (aynı yüzey, `R_XL`, kenarlık, `float_shadow`; yerleşikte `dock_shadow`); yerleşikken yüzene daralınca kapanır. Eni 360 px (yüzen panelle aynı), sağda 16 px. **Yükseklik içerik kadar** (başlık şeridi + gruplar + bloklar; uzunsa kendi kaydırması ve alt / üst kenar solması, altında boş kart yok) ve hapın ÜSTÜNDE biter (24 px boşluk); hapı daraltmaz (Şimdi çalıyor paneli hapın yanında pencerenin altına kadar iner ve hapı daraltır). Üç sekmenin başlık şeridine dördüncü hap sığmıyor, bu yüzden panel kendi başlığıyla açılır: sol üstte "Arkadaş etkinliği" (Body, kalın), sağda kapat. Kapatınca, yerleşik "Şimdi çalıyor" paneli açık tercihteyse yerine o döner. Arkadaş panelinin açılıp kapanması "paneli açık başlat" tercihini değiştirmez.
- **Açma düğmesi:** başlık çubuğunun sağında, pencere düğmelerinin solunda (eşitleme göstergesinin yanında) 36 px'lik açılır düğme (`users`, Lucide; panel açıkken kapak ışığı renginde + altında 4 px nokta, `toggle_button`), ipucu "Arkadaş etkinliği". Yalnız ayar açıkken görünür; Tab sırasında aramadan sonra (`tab::FRIENDS`).
- **Gruplar:** "Şimdi dinliyor" (canlı, `is_live`: son 3 dk) ve "Daha önce"; boş grubun başlığı yok. Grup adı sıra panelinin bölüm başlığıyla aynı yazı (Label, yarı kalın, etiket tonu; liste grubu adı, eyebrow değil), 32 px; ikinci gruptan önce 12 px boşluk. Sıra: önce canlılar, sonra eskiler; her grupta en yeni önde.
- **Blok** (iki satır, dikey ritim 8 px, 6 px köşe, panel iç boşluğu 8 px; yükseklik 100 px, bağlam satırı varsa 106): **A satırı:** 32 px yuvarlak profil resmi (`image`, `cover_image`; yoksa ya da inmediyse baş harf, adın FNV-1a özetinden 24 adımlı ton çemberinden seçilen sakin bir tonda daire: zemin `PLACEHOLDER` yüzeyine tonun %30 karışımı, harf aynı tonun açığı ≥ 4,5:1; aynı ad hep aynı ton), arkadaşın adı (Body, yarı kalın), sağ uçta zaman (Label, eşit genişlikte rakam, etiket tonu: "3 dk" …; canlıysa önünde kapak ışığı renginde 6 px nokta; üzerine gelince mutlak zaman "Nov 12, 2026, 19:39" ipucu). **B satırı** (adın altına girintili, 40 px yerine 48 px: üç satırlık yazı 54 px olduğundan 40 px'lik kapak yazıdan kısa kalıyordu): 48 px şarkı kapağı (`R_SM`, buddylist'in şarkı `imageUrl`'i, `Friend::track_image`; yoksa `music` yer tutucu), yanında en çok üç satır: şarkı adı (Body, ön plan; tek satır "…", tamamı ipucunda), sanatçı (Secondary, soluk; tıklayınca sanatçı sayfası, üzerine gelince alt çizgi), bağlam (Label, etiket tonu; liste `list-music`, albüm `disc-3`, sanatçı `user` küçük simgesi + bağlam adı; liste / albüm / sanatçı bağlamıysa tıklayınca o sayfa, başka türde düz yazı). Üzerine gelme / basma `hover_fill` (%6 / %14) bütün blokta, klavye odağı 2 px kapak ışığı halkası + %10 katman. Kapağın üzerine gelince ortada 32 px koyu daire içinde `play-fill` (ipucu "Çal").
- **Tıklama:** blok (ve kapak) şarkıyı bağlamıyla çalar; ad (tıklama ya da sağ tık) menüyü açar: bağlam liste, albüm ya da sanatçıysa o bağlamın şarkıları arkadaşın şarkısından başlayarak (şarkı bağlamda adresiyle aranır; birden çok yerde geçiyorsa yanıttaki sıraya en yakını; bağlamda bulunamazsa ya da alınamazsa YANLIŞ şarkıdan başlamak yerine şarkı tek başına); bağlam yoksa ya da çözülemeyen türdeyse (beğenilenler, radyo …) yalnız şarkı.
- **Sağ tık menüsü** (mevcut menü deseni, `manage::Act`): Çal, Albüme git, Sanatçıya git, ayırıcı, "Profili Spotify'da aç". Şarkının kendi sayfası uygulamada olmadığı için "şarkıya git" albüm sayfasını açar (`MenuGoAlbum`). Yüzen panelden bir sayfaya gidilince panel kapanır (içeriği örtüyor), yerleşik panel kalır. Shift+F10 odaktaki satırın menüsünü açar; Esc önce menüyü kapatır.
- **Klavye:** her satır ve kapat düğmesi `tab::PANEL` durağı (Tab / Shift+Tab; Enter / Boşluk çalar; ↑ / ↓ komşu satıra; Tab ile görünür alan dışındaki satır panelin içinde kaydırılarak görünür kılınır).
- **Hâller (panelin ortasında `state_view`: 48 px simge karosu `users`, başlık Body yarı kalın, tek cümle Secondary soluk; panel kısa bir kart olur, 208 px gövde):** yükleniyor ("Arkadaş etkinliği yükleniyor…" / "Bu birkaç saniye sürer."), boş ("Gösterilecek arkadaş etkinliği yok" / "Dinlediklerini paylaşan arkadaşların burada görünür."), alınamadı ("Arkadaş etkinliği alınamadı" / "Bağlantını kontrol et. Biraz sonra yeniden denenecek."), Spotify yoğun ("Spotify şu an yoğun" / "Birkaç dakika sonra yeniden denenecek."), bu hesapta yok ("Bu hesapta arkadaş etkinliği kullanılamıyor" / "Spotify bunu bu hesapla paylaşmıyor."). Eski bir liste varsa hata sırasında o gösterilmeye devam eder (hata ekranı yalnız hiç liste yokken çıkar).
- **Yenileme:** yalnız ayar açık, panel ekranda ve pencere tepsiye gizli değilken (tam ekran da sayılmaz), açılışta hemen sonra 60 sn'de bir; gizlenince / panel kapanınca durur, açılınca süre dolduysa hemen sorar, dolmadıysa kalanı kadar bekler. Hata ya da 429'da aralık katlanır (2, 4, 8, en çok 10 dk); 429'da en az 5 dk; 401 / 403 / 404'te 10 dk. İstek, token ve kapı (2 sn aralık, Retry-After, iki 429'da devre kesici, 401'de token yenileme, 5xx yeniden denemesi) `pathfinder.rs`'in ortak altyapısından geçer; kapısı ayrı bir örnektir (`pathfinder::presence`): buradaki 429 aramayı ya da Keşfet'i durdurmaz. Sonuçlar yalnız bellekte durur, diske yazılmaz; ayar kapanınca bellekteki liste de atılır.
- **Sınama:** `OPENIFY_TEST_FRIENDS_JSON=<dosya>` (`docs/TESTING.md`), hazır fikstür `src/engine/fixtures/friend_activity_demo.json`; ekran sınaması `OPENIFY_TEST_VIEW=friends`. README görüntüsü için `src/engine/fixtures/friend_activity_readme.json` (İngilizce; `seed_readme_library` kapaklarına bağlanır: `demo.invalid/covers/artist-N` profil, `album-N` şarkı kapağı).

## Kütüphane düzeni (12 Kasım)

Klasörler, sabitleme, sıralama ve eklenme tarihi (`src/ui/library_layout.rs`, `library_tree.rs`, `added_date.rs`); sakin: yeni renk, gölge ya da eyebrow başlık yok, hepsi mevcut belirteçlerle.

- **Klasör (salt okunur):** kapak yerinde `PLACEHOLDER` ton karosu üstünde soluk klasör simgesi (Beğenilenler'in kalbi gibi), altyazı "N liste", sağ tık menüsünde yalnız Aç / Genişlet / Daralt. **Liste görünümü:** 56 px satırın başında 16 px ok yeri (klasörde ▸ / ▾, listelerde boş: kapaklar hizalı), klasörün içindekiler 24 px (`S6`) girintili, satıra tıklamak açıp kapar; durum `library_open_folders` (kalıcı kimlik). **Izgara:** klasör kartı içine girer; üstte yol çubuğu: geri düğmesi (Tab durağı, ipucu "Bir üst klasör · Esc") ve "Kütüphane › Yol › Spor" (son parça kalın, öncekiler tıklanır), boş klasörde sakin durum görünümü.
- **Sabitleme:** sağ tık "En üste sabitle" / "Sabitlemeyi kaldır" (iğne simgeleri). Sabitli öğe en üstte; kapakta sol üstte 16 px (şeritte 16, rafta 24) yarı saydam daire içinde iğne (sağ alttaki "çalıyor" simgesiyle karışmaz), listede sağda küçük iğne. Kökte sabitliler klasörün içindekiler dahil en üstte ayrı blok.
- **Sıralama menüsü:** düğme (simge + geçerli sıralama) tıklanınca altında menü (`menu_panel`): Son çalınan, Eklenme tarihi, Alfabetik, Yaratıcı; geçerli olan solda onay işaretiyle (öbürlerinde simge yeri boş, hizalı); ↑ / ↓ + Enter, Esc. Seçim kalıcı.
- **Eklenme tarihi sütunu:** şarkı tablosunda albümden sonra, kalpten önce, 132 px, ikincil renk; "Az önce / 5 dk önce / 3 saat önce / 3 gün önce / 2 hafta önce", 30 günden eskiyse "3 Kas 2025" ("Nov 3, 2025"). İçerik 1000 px'ten darsa sütun ve başlığı gizli (şarkı ve albüm sütunları genişler).
- **Klavye:** kütüphane sayfasında ok tuşları kartları gezer (halka kapak ışığı renginde), Enter açar; listede → klasörü açar, ← kapatır ya da üst klasöre geçer; ızgarada Esc / Backspace bir üst klasöre.

## Kütüphane araçları (12 Kasım)

Tek sayfa (`Route::Tools`), Kütüphane sekmesinin altında: Kütüphane araç çubuğundaki "Araçlar" düğmesi (dar pencerede yalnız simge) ve palet komutu açar. Yapı ayar sayfasıyla aynı sakinlikte: sayfa başlığı, altında beş aracın seçenek düğmesi (`segmented_track`; ← / → gruptaki komşuya), altında seçili aracın gövdesi tek sütun, en çok 760 px.

- **Seçim kutusu:** koyu, ince kenarlı, köşe `R_LG`; 7 satır (40 px), uzunsa kendi kaydırması (tekerlek ve ↑ ↓ Home End Sayfa tuşları; imleç hep görünür). Kutu TEK Tab durağı (odakta kenar vurgu rengi); Boşluk / Enter imleçteki satırı seçer, fareyle tıklayınca da. İşaret: çoklu seçimde kare, tekil seçimde yuvarlak, seçiliyken vurgu rengi dolu ve içinde onay işareti. Eyebrow / kicker başlık yok; adım başlıkları normal yazım, `T_BODY` yarı kalın.
- **Yinelenen tablosu:** aynı kutu; küme başlığı satırı (tür + "N kaldırılabilir"), altında şarkı satırları: onay kutusu (kalan satırda iğne simgesi ve "Kalır"), ad, sanatçı, albüm (1000 px üstü), süre. Kalmayacak (işareti kaldırılmış) satır soluk.
- **Önizleme ve onay:** birleştir / böl'de ayrı adım (önizleme paneli: ad alanı, özet, ilk 6 satır ya da liste, "Kaynak listeler olduğu gibi kalır", Oluştur / Geri). Ad alanı `select_field` boyutunda, odakta ince imleç çizgisi.
- **Toplu işlem (≥ 50 değişiklik):** eylem düğmesinin yerine vurgu kenarlı, vurgu tonlu onay kartı: kaç değişiklik, geri alınabilir uyarısı, Uygula / Vazgeç (Esc vazgeçer, Enter uygular).
- **İstatistik:** üç aralık çipi; iki büyük sayı (çalma, süre; `T_HEADLINE`); saat dağılımı 24 sade sütun (en yoğun saat vurgu renginde, diğerleri vurgunun saydam tonu; 0 / 6 / 12 / 18 etiketi); üç liste (şarkı, sanatçı, albüm) 5 satır, "Tümünü göster" ile 10. Boş durum `state_view`.
- Boşluk, köşe, yazı ve renk yalnız `theme.rs` belirteçlerinden (`design_rules.rs` iki dosyayı da tarar).

## Radyo, şarkıyı gizle, gizli oturum (12 Kasım)

Üç özellik de mevcut menü, satır ve ayar desenlerinin içinde; yeni renk, gölge, eyebrow başlık ya da panel yok. Simgeler Lucide: `radio`, `eye-off`, `eye`.

- **Radyoyu başlat:** şarkı satırı menüsünde (tek şarkı seçiliyken; "Albüme git"ten önce), liste / albüm / sanatçı kartı menüsünde ("Çal"dan sonra; Beğenilenler'de ve klasörde yok) ve palette ("Çalan şarkının radyosunu başlat", şarkı yokken soluk). Tıklayınca kısa bildirim ("Radyo başlıyor…"); hata olursa bir `Problem` bildirimi (genel / Spotify sınırlıyor / bu hesapta yok). Radyo çalarken sıra panelinin ikinci bölüm başlığı "Sıradaki: <ad> radyosu" olur (liste için "Sıradaki: <liste>" gibi; ad yoksa "Sıradaki: radyo"); sıra sonsuzdur, panelde ayrı bir görünüm yok.
- **Şarkıyı gizle:** şarkı satırı menüsünde ("Bu şarkıyı gizle": yalnız bu liste / albüm; "Hiçbir yerde çalma": genel; gizliyken tek "Göster"). Çok seçimde sayılı ("N şarkıyı gizle"). Gizli şarkının satırı %42 donukluğa iner (okunur kalır, kalp / sağ tık / çift tık çalışır: elle seçilen şarkı çalar) ve karıştırmada, Sonraki / Önceki'de, tekrarda atlanır. Listeyi "Çal" düğmesiyle baştan çalarken ilk gizlenmemiş şarkıdan başlar. Ayarlar → Karıştırma ve sıra'da "Gizlenen şarkılar" başlık satırı (açıklama yerel olduğunu söyler) ve en yeni önde en çok 30 kayıt: ad, altında "sanatçı · Her yerde" ya da "sanatçı · <liste> içinde", sağda "Göster" düğmesi; boşken tek cümle yönlendirme; 30'dan fazlaysa "ve N tane daha". Hepsi klavye durağı.
- **Gizli oturum:** profil menüsünde "Gizli oturum" (Ayarlar'dan sonra; açıkken simge `eye-off` ve satır sonunda "Açık"), palette ("Gizli oturumu aç" / "Gizli oturumu kapat"), Ayarlar → Görünüm ve davranış'ta anahtar. Açıkken başlık çubuğunun sağında, arkadaş düğmesinin solunda 36 px'lik `eye-off` düğmesi (vurgu renginde değil, normal simge düğmesi; ipucu "Gizli oturum açık. Kapatmak için tıkla."; tıklayınca kapatır; Tab sırasında aramadan sonra). Açılırken bildirim: "Gizli oturum açık. Openify kendi geçmişine ve istatistiğine yazmaz." Dürüstlük: yazı yalnız Openify'ın kendi geçmişinden söz eder (Spotify'a zaten dinleme bildirilmez); uygulama kapanınca biter (kalıcı değil).
- **Dar pencere (900 px):** menüler ve ayar satırları mevcut genişliklerinde; başlık çubuğu düğmesi arama kutusunun daraldığı genişlikte de sığar.
