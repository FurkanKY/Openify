# gpui (Openify kopyası)

Bu klasör [Zed](https://github.com/zed-industries/zed) projesinin `gpui` paketinin 0.2.2 sürümüdür (crates.io; kaynak `crates/gpui`, işleme `69e2130`). Openify `Cargo.toml` içindeki `[patch.crates-io]` ile bu kopyayı kullanır. Değişen her satır `OPENIFY:` açıklamasıyla işaretli; asıl sürümle karşılaştırmak için:

```
diff -ru ~/.cargo/registry/src/index.crates.io-*/gpui-0.2.2 vendor/gpui
```

## Lisans

Apache-2.0. Telif: Zed Industries. Lisans metni `LICENSE-APACHE` dosyasında.

## Değişiklikler

Hepsi Windows katmanında (`src/platform/windows/`), genel API değişmedi. Bir de `src/taffy.rs`'te tür belirtme, `src/scene.rs`'te gölgenin çizim sırası (madde 7) ve `src/elements/div.rs`'te bekleyen ipucunun tıklamayla iptali (madde 9); bunlar bütün platformlar.

### 1. Görünür pencere yokken ekran yenilemesi iş parçacığı uyur (`vsync.rs`, `platform.rs`, `events.rs`)

- **Sorun:** gpui'nin `VSyncProvider` iş parçacığı her ekran yenilemesinde `DwmFlush` bekleyip bütün pencerelere `RedrawWindow` gönderiyordu, pencereler gizli olsa da. Openify tepside gizliyken süreç bu yüzden çekirdeğin %0,5–1,2'sini harcıyordu (iş parçacığı başına ölçümde hepsi `VSyncProvider`).
- **Yama:** döngünün başında pencere listesine bakılır; hiçbiri görünür değilse (gizli ya da simge durumunda) iş parçacığı `std::thread::park_timeout` ile uyur. Gizli pencere zaten `WM_PAINT` almaz, simge durumundakinin çizim geri çağrısını gpui kendisi bekletir; yani uyku hiçbir kareyi kaçırmaz.
- **Uyandırma:** pencere iletisi `WM_WINDOWPOSCHANGED`, `WM_SHOWWINDOW` ya da `WM_SIZE` gelince ve yeni pencere listeye eklenince `unpark`. `unpark` uykudan önce gelirse jeton saklanır, uyandırma kaybolmaz. `WM_SHOWWINDOW` görünürlük değişmeden önce geldiği için asıl uyandırma `WM_WINDOWPOSCHANGED`'dir (sonra gelir). Yine de kaçan bir yol olursa diye uyku en fazla 1 sn sürer (`VSYNC_IDLE_RECHECK`); o an iş parçacığı yalnız görünürlüğe bakıp yeniden uyur (ölçümde işlemci 0 ms).
- Görünür pencere varken davranış aynı: her karede eklenen iş, 3 pencere için `IsWindowVisible` + `IsIconic`.

### 2. Gizli pencerenin çizim yüzeyleri bırakılır (`directx_renderer.rs`, `events.rs`)

- **Sorun:** her gpui penceresi pencere boyunda yüzeyler tutar: takas zincirinin 2 arabelleği, yolların ara dokusu ve 4× MSAA dokusu (1180×760 pencerede yaklaşık 7 × 3,6 MB). Ekran kartı sürücüsü (AMD, D3D11) bunların bir kısmını süreç belleğinde (yazma-birleştirmeli) tutuyor; pencere gizliyken de duruyordu.
- **Yama:** pencere gizlenince (`WM_WINDOWPOSCHANGED` + `SWP_HIDEWINDOW`) `DirectXRenderer::release_surfaces` bu yüzeyleri 1×1'e küçültür (gpui'nin kendi boyutlandırma yolu: `ResizeBuffers` + dokuları yeniden kurma) ve `Flush` ile sürücünün ertelediği serbest bırakmayı hemen yaptırır. Bağlamın 0. doku yuvası da boşaltılır (yoksa son çizilen ara doku bağlı kalıp bırakılamıyor).
- **Gösterme:** `WM_SHOWWINDOW` (ya da `SetWindowPos(SWP_SHOWWINDOW)` yedeği olarak `WM_WINDOWPOSCHANGING`) gelince `restore_surfaces` yüzeyleri eski boyuta (gizliyken boyut değiştiyse yeni boyuta) döndürür ve son sahne `require_presentation` ile yeni arabelleklere çizdirilir (görünüm kirliyse tam kare). Atlas, boru hatları ve sahne olduğu gibi kaldığı için yeniden kurma / yerleşim yok.
- **Uç durumlar:** gizliyken gelen boyut değişimi yalnız dönülecek boyutu günceller. Simge durumunda gösterilen pencerenin yüzeyleri simge durumundan dönüşte (`WM_SIZE`) geri alınır. Güvenlik için `DirectXRenderer::draw` yüzeyler bırakılmışsa önce geri alır: hiçbir yolda 1×1 arabellek gösterilmez. Durum o an ödünç alınmışsa (`RefCell`) bırakma / geri alma atlanır, ilk çizim geri alır.

### 3. Kapanan pencerenin yüzeyleri hemen yok edilir (`directx_renderer.rs`, `Drop for DirectXRenderer`)

- **Sorun:** D3D11 nesneleri ertelenerek yok eder; bağlam bütün pencerelerde ortak olduğu için kapanan pencerenin yüzeyleri başka bir pencere çizip `Flush` olana kadar (hiç olmayabilir) bellekte kalıyordu. Pencere aç / kapa başına ~0,7 MB kalıyordu.
- **Yama:** bırakmada önce kompozisyon nesneleri (takas zincirini tutuyor), sonra doku yuvası ve çizim hedefi boşaltılıp yüzeyler bırakılır, ardından `Flush`.

### 4. `src/taffy.rs`: `length(0.0_f32)`, `fr(1.0_f32)`
Yol bağımlılığı olunca görünen iki "falling back to `f32`" uyarısı. Davranış aynı (tür zaten f32'ye düşüyordu).

### 5. `Window::release_memory()`: gizlenirken kareler arasında tutulan fazlalık bırakılır (`window.rs`, `scene.rs`, `bounds_tree.rs`, `arena.rs`)

- **Sorun:** gpui her pencerede iki kare (`rendered_frame`, `next_frame`) ve bir iş parçacığına bir elemanlar arenası tutuyor; vektörleri (sahne: dörtgen / sprite / yol listeleri, isabet kutuları, fare dinleyicileri…) kareler arasında `clear()` ile boşaltıp kapasitesini saklıyor, arena de 1 MB'lık parçalarını geri vermiyor. Kapasite en ağır kareye göre büyüyor ve hiç küçülmüyor (gizliyken ~3 MB).
- **Yama:** yeni genel API `Window::release_memory`. İki karenin vektörlerini `shrink_to_fit` yapar (`Frame::shrink`, `Scene::shrink`, `BoundsTree::shrink`); son çizilmiş sahnenin içeriği olduğu gibi kalır (gösterilince bu sahne yeniden sunuluyor, bkz. 2), yalnız boş kapasite gider. Arena (`Arena::shrink`): boşaltılır, ilk parça dışındakiler ve eleman listesi bırakılır; çizim sırasında çağrılırsa (`try_borrow_mut` başarısızsa) arenaya dokunulmaz. Ardından `PlatformWindow::flush_gpu` (yeni, varsayılanı boş; yalnız Windows DirectX'te bağlama `Flush` atar): kapanan pencerelerin nesne yok etmelerini sürücü ancak sonraki bir `Flush`'ta bitiriyor (aşağıya bak).
- **Çağıran:** Openify, ana pencere gizlenince (ve gizliyken açılan palet / menü kapanınca) 1,5 sn sonra, bir kez daha 2 sn sonra (`src/ui/integration.rs` `schedule_trim`); gizliyken açılır pencereler (palet, tepsi menüsü) yok edilir (bir sonraki açılış pencereyi yeniden kurar), kalan pencerelere `release_memory` uygulanır, NT yığınları sıkıştırılır (`HeapCompact`: tek başına kazanç **0**, ölçüldü; zararsız olduğu için kaldı, istenirse `OPENIFY_TRIM_SKIP=heap` ile atlanır).
- **Gösterme:** hiçbir şey gerekmiyor; vektörler ilk karede gereken kadar yeniden büyür. Gösterilen ilk kare gizlenmeden önceki kareyle piksel piksel aynı (PrintWindow, y 100–640: 0 / 645 840 piksel fark; tüm pencerede 102 piksel: ilerleme çubuğu).

### 6. Kapanan pencerenin bağlam bağları da bırakılır (`directx_renderer.rs`, `unbind_texture_slot`)

- 2. ve 3. maddelerdeki Flush, bağlamda **bağlı kalan** nesneleri (doku yuvası 0 dışındaki SRV'ler, gölgelendiriciler, sabit tamponlar, harmanlama durumu) serbest bırakamaz; çizim her çağrıda bunları yeniden bağlıyor, yani bırakmak güvenli. `unbind_texture_slot` artık 8 SRV yuvasını, 2 sabit tampon yuvasını, gölgelendiricileri ve harmanlama durumunu boşaltır. Örnekleyici, rasterleştirici ve görüntü alanı başka pencerelerin tek seferlik kurulumu olduğu için dokunulmaz (`ClearState` bu yüzden kullanılmadı).
- Ölçülen kazanç küçük (en ağır durumda 75,0 → 74,7 MB); asıl belirleyici, kapanan pencerenin nesne yok etmelerinin ikinci bir `Flush`'la (5'teki 2 sn sonraki geçiş) tamamlanması: en ağır durumda 75 → 71 MB.

### 7. Gölgenin çizim sırası bulanık payıyla birlikte verilir (`scene.rs`, `Scene::insert_primitive`)

- **Sorun:** gölge, sınırının `3 × blur_radius` dışına kadar boyar (`shaders.hlsl` `shadow_vertex`: `margin = 3.0 * blur_radius`), ama çizim sırası (`BoundsTree`) yalnız gölgenin kendi sınırıyla veriliyordu. Bulanık payın altında kalan, gölgeden ÖNCE boyanmış ama sırası gölgeninkinden büyük öğeler (iç içe kutular, satır yazısı, kapak; sıra, iç içelik derinliğiyle büyür) gölgenin ÜSTÜNE çiziliyordu: gölgede keskin kenarlı, gölgesiz yamalar.
- **Yama:** gölgenin sıra kutusu `bounds.dilate(3 * blur_radius)`: boyadığı alanın tamamı. Sonuç boyama sırasıyla aynı (önce boyanan altta, sonra boyanan üstte); ek iş yok. İçerik maskesinin boşluk denetimi de bu alanla yapılır (öğesi maskenin dışında kalıp bulanığı içeri taşan gölge artık atılmıyor).
- **Kanıt (bağımsız gpui örneği, 3 panel + aralarında 8 kat iç içe kutular):** önce kutular gölge payında ham renginde (#3A3A40), sonra gölgenin altında (#333339); 52 544 piksel değişti, yalnız gölge paylarında. Openify'ın kendi ekranlarında fark küçük (vitrinde panel ile hap arasında 4 757 piksel, en çok 7 düzey).
- **Kare süresi (`kare-ab.ps1` wheel, 4 tur dönüşümlü, yalnız bu yama farklı iki exe):** önce p50 4,88 / p95 7,71 ms, 130,7 kare/sn; sonra p50 4,81 / p95 7,57 ms, 132,5 kare/sn: gerileme yok. (Makine o sırada başka derlemelerle yüklüydü; mutlak değerler sakin makinedeki ~3,8 ms p95'ten yüksek, iki değişken aynı koşulda.)

#### "Sağ panelin gölgesi şeffaf delik açıyor" (7 Kasım): gpui'de değil, ekran görüntüsü betiğinde

- Sağ panelin gölgesi (ve iki katmanlı gölgeler) PrintWindow görüntülerinde panelin iki kenarında 4–5 × 7–8 px'lik (0,0,0,0) bloklar bırakıyordu; panel bu yüzden gölgesiz yapılmıştı. Vitrinde (`OPENIFY_GALLERY=golge`, `OPENIFY_GOLGE_KATMAN=0|1|2`) aynı panelle yeniden üretildi: tek katmanda 136 piksel, gölgesizde 0.
- Ölçülen: arka arabellek (çizimden hemen sonra GPU'dan okundu) o piksellerde **doğru**: (13, 11, 12, 255), yani #0D0B0C. Görüntüdeki 136 delik pikselin hepsi ve yalnız onlar bu renkte. gpui'siz bir WinForms (GDI) penceresine boyanan #0D0B0C de aynı şekilde (0,0,0,0) çıktı, komşu renkler (#0E0B0C, #0C0B0C, #0D0B0D) çıkmadı.
- Neden: betikler PrintWindow'u `System.Drawing.Graphics.GetHdc()`'ye çizdiriyordu; GDI+ bu yolda #0D0B0C'yi kendi saydamlık anahtarı sayıyor (32 bpp ARGB'de (0,0,0,0), RGB biçimlerinde siyah). Gölgenin yumuşak geçişi bu renge tam denk gelen bantlar üretince "delik" göründü; ekranda hiç yoktu. İki katmanlı gölgede renkler kaydığı için o örnekte yoktu.
- Çürütülen başka adaylar (hepsi denendi, delik aynen kaldı): gölgelendirici çıktısında NaN / sonsuz (bütün parça gölgelendiricilerine sonluluk koruması), temizleme rengi (magenta ile temizlendi: delik magenta olmadı), alfa harmanlaması (`DestBlendAlpha = INV_SRC_ALPHA`), takas zincirinin kullanımı (UAV / SRV), DirectComposition (`GPUI_DISABLE_DIRECT_COMPOSITION=1`). Hepsi geri alındı; gpui'de bu konu için değişiklik yok.
- Düzeltme betikte: `scripts/ekran-sinama.ps1` PrintWindow'u düz bir GDI bitmap'ine alıp `Image.FromHbitmap` ile kaydediyor (aynı pikseller #0D0B0C, delik 0). `gizli-olcum.ps1`, `kaydirma-sinama.ps1`, `sistem-sinama.ps1` hâlâ `GetHdc` kullanıyor: #0D0B0C pikselleri onlarda da siyah / saydam görünür.

## Ölçümler (7 Kasım 2026, AMD Radeon RX 5700, tek ekran 1920×1080 %100, sınama penceresi 1180×760)

Önce / sonra aynı kaynaktan iki exe (yalnız gpui farklı), dönüşümlü.

| ölçüm | önce | sonra |
|---|---|---|
| tepside gizli, süreç işlemcisi 60 sn (`is-parcacigi-islemci.ps1 -Gizle -Calma`) | 312 ms = %0,52 (`VSyncProvider` 297 ms) | **0 ms = %0** |
| aynı, açılır pencereler bir kez açılmış (`-Acilir`) | 484 ms = %0,81 | **0 ms = %0** (2 koşu) |
| simge durumunda 10 sn, süreç işlemcisi | 281 ms | 31 ms (iki saat tıkı; iş parçacığı ayrımı ölçülmedi) |
| tepsiden gösterme → pencere görünür (2 × 15 tur, dönüşümlü) | ortanca 12,1 / 24,1 ms, en çok 29,5 | ortanca 12 / 13 ms, en çok 27,3 |
| gizli özel bellek (açılır pencere açılmadan) | 84,3–84,5 MB | **64,9–66,3 MB** |
| gizli özel bellek (palet + tepsi menüsü bir kez açılmış) | 102,5–103,0 MB | **71,7–71,9 MB** |
| gizli özel bellek (palette arama yapılmış + menü, 2 tur) | 111,3–111,7 MB | **78,3–80,8 MB** |
| gizli özel bellek, 5.–6. maddelerden sonra (en ağır durum: palette arama + menü, 3 tur dönüşümlü; normal durum) | en ağır 80,7–81,2 MB; normal 66,8–67,1 MB | **en ağır 70,4–71,7 MB; normal 64,0–64,6 MB** |
| gizliyken palet / tepsi menüsü açılışı (pencere her seferinde yeniden kurulur) | 2,6–3,5 ms (aynı pencere yeniden kullanılırdı; yalnız ana pencere görünürken hâlâ öyle) | 15,5–16,2 ms (ilk açılış 28–32 ms); 12 + 12 yeniden kuruluşta bellek birikmedi (67,2 → 67,5 MB) |
| pencere aç / kapa başına kalan bellek (40 pencere, `gpui-sizinti`) | 724–727 KB | **61–69 KB** |
| görünürken kare (`kare-ab.ps1 play`, 6 tur ortancası) | 25,6 kare/sn, %6,7, çizim p50 0,74 / p95 1,00 ms | 26,4 kare/sn, %6,5, p50 0,64 / p95 1,09 ms |
| görünürken kare (`kare-ab.ps1 wheel`, 6 tur ortancası) | 68,3 kare/sn, %31,8, p50 3,18 / p95 4,28 ms | 68,2 kare/sn, %30,7, p50 3,15 / p95 4,24 ms |
| tepsiden gösterme, açılır pencerelerin ikinci açılışı | görüntüler | piksel piksel aynı (PrintWindow, palet / yazılı palet / menü) |

Bellek, gizlendikten 2–3 sn sonra düşüyor (sürücü serbest bırakmayı o kadar erteliyor). Gösterilince yüzeyler geri gelir (görünürken bellek öncekiyle aynı).

## Kalan bellek (gizli; 5.–6. maddelerden önce ~72 MB, sonra normal durumda 64 MB, döküm `bellek-dokum.ps1` sınıflarıyla)

- Rust / DLL yığını (NT heap) ~23 MB + büyük yığın blokları ~16–18 MB (bunun çoğu işlenmiş ama hiç dokunulmamış sayfa). Openify'ın kendi verisi (kütüphane, kapaklar, SQLite) ve gpui'nin kare arenası (1 MB'lık parçalar, geri verilmez).
- Sürücü (yazma-birleştirmeli) ~20–24 MB: AMD sürücüsünün yükleme / komut halkaları. `IDXGIDevice3::Trim` denendi: ~0,8 MB kazanç, ama ortak bağlamda `ClearState` gerektiriyor; değmez bulundu, eklenmedi. Boru hatlarının örnek tamponlarını küçültmek de denendi: kazanç yok.
- Yazılabilir görüntü bölgeleri (DLL'ler) ~6 MB, iş parçacığı yığıtları ~2 MB.

### 8. Sınama kancası: zorlanan ekran ölçeği ve ekrandan büyük pencere (`window.rs` `test_scale_override`, `events.rs`)

- **Sorun:** tek %100'lük 1920×1080 ekranlı makinede %125 / %150 / %200 ekran ve 2560×1440 pencere görünümü sınanamıyordu (gpui pencere ölçeğini ekranın DPI'ından alır; ekrandan büyük pencere de ikinci ekrandan taşıp birinci ekrandaki işe biner).
- **Yama (yalnız `OPENIFY_TEST_DISPLAY` + `OPENIFY_TEST_SCALE=<0,5–4>` ikisi de varken; normal çalışmada hiçbir şey değişmez):** (a) pencere ölçeği (`WindowsWindowState::new`, `WM_DPICHANGED`) bu değerdir; (b) `WindowKind::Normal` ana pencere `retrieve_window_placement`'te mantıksal boyut × ölçek kadar CİHAZ pikseliyle, ekrana sığıyorsa ekranın ortasına, sığmıyorsa sanal masaüstünün sağ kenarının ötesine (`x = SM_XVIRTUALSCREEN + SM_CXVIRTUALSCREEN + 64`) konur: hiçbir ekranda görünmez, `PrintWindow` yine tam görüntüyü alır; (c) `WM_GETMINMAXINFO` üst sınırı 16384. `scripts/ekran-sinama.ps1 -Scale 1.5`.
- **Sınır:** yalnız ölçek çarpanı taklit edilir (yazı, resim, 1 px çizgiler gerçekten o ölçekte çizilir); pencere çerçevesi / başlık düğmesi gibi Windows'un kendi DPI ölçüleri (`GetSystemMetricsForDpi`) gerçek %100 kalır, ölçekli ekranda başlık çubuğunu bizim çizdiğimiz için görüntüye yansımaz.

### 9. Tıklama, henüz görünmemiş (bekleyen) ipucunu iptal eder (`elements/div.rs`, `clear_active_tooltip_if_not_hoverable`)

- **Sorun:** düğmeye tıklayınca açılan menü ya da pencerenin üstünde, 500 ms sonra düğmenin ipucu beliriyordu (üstelik eski kapanışın yazısıyla: tıklamayla durum değişmişti). `clear_active_tooltip_if_not_hoverable` tıklamada (ve tekerlekte) yalnız görünmekte olan, üzerine gelinemeyen ipucunu siliyor; `WaitingForShow` (gösterilmeyi bekleyen) duruma dokunmuyordu, zamanlayıcı yine ateşleniyordu.
- **Yama:** `Some(ActiveTooltip::WaitingForShow { .. }) => true`: bekleyen ipucu da silinir, zamanlayıcı `Task`'ı düşer ve ipucu hiç gösterilmez. Üzerine gelinebilen görünür ipucu (`is_hoverable`) ve `WaitingForHide` eskisi gibi kalır.
- **Etki:** çağıranlar `register_tooltip_mouse_handlers`'ın fare-basma ve tekerlek dinleyicileridir. Bekleyen ipucunun `tooltip_id`'si yoktur (`set_tooltip_on_window` onu `None` döndürür), bu yüzden `!is_hovered` koşulu hep sağlanır ve öğenin kendisine tıklamak da bekleyeni iptal eder; ipucu fare yeniden oynayınca (yeni `MouseMove`) yine kurulur. Görünür ipucunun davranışı değişmedi, fareyi bekletip ipucunu görenin gecikmesi aynı. Menü ve pencere katmanlarının arkadaki düğmeyi "üzerinde" saymaması ayrıca uygulamada çözüldü (`occlude`; bkz. `DESIGN.md`).

## Riskler

- Gösterme yolu `WM_SHOWWINDOW` / `WM_WINDOWPOSCHANGING`'e dayanıyor; Windows'un pencereyi bu iletiler olmadan gösterdiği bir yol olursa VSync en geç 1 sn içinde uyanır ve ilk çizim yüzeyleri geri alır (siyah kare gösterilmez, ama o pencere kirlenene kadar eski içerik gelmeyebilir).
- Her gizle / göster bir `ResizeBuffers` ve bir sunum ekliyor; gösterme süresi ölçümde değişmedi (yukarıdaki tablo). Makine yüklüyken tek koşularda iki sürümde de 40–180 ms görüldü (önce 44–80, sonra 61–180); dönüşümlü 15'er turda fark yok. Açılır pencereyi çok sık aç / kapa yapınca (250 ms aralık) sürücü eski arabellekleri geç bıraktığı için o an bellek öncekinden yüksek görünebilir (sistem-sinama palet adımında açıkken 124,6 MB'a karşı 105 MB); gizlendikten sonra düşüyor.
- Başka sanal masaüstündeki (cloaked) ya da tamamen örtülü pencere "görünür" sayılır: VSync o durumda eskisi gibi çalışır.
- `Window::release_memory` arenayı boşaltıyor (`Arena::shrink` → `clear`): çizim sırasında çağrılırsa canlı `ArenaBox`'ları geçersiz kılardı. `try_borrow_mut` yalnız çizim içinden çağrıyı yakalar; Openify yalnız `AsyncApp` güncellemesinden (çizim dışı) çağırıyor. Başka bir çağıran çizim sırasında çağırmamalı.
- Gizliyken açılır pencere kapanınca yok edildiği için bir sonraki palet / menü açılışı pencereyi yeniden kurar (15,5–16 ms; çıta 100). Pencere başına aç/kapa kalıntısı 61–69 KB (madde 3); gizliyken en sık kullanımda bile (12 palet + 12 menü) bellek birikmedi. Ana pencere görünürken açılır pencereler eskisi gibi yeniden kullanılıyor.
- `flush_gpu` ve ikinci bırakma geçişinin kazancı (≈4 MB) sürücüye bağlı (AMD Radeon RX 5700 / D3D11'de ölçüldü); başka sürücüde daha az olabilir, zararı yok.

İyileştirmeler asıl projeye de önerilebilir.
