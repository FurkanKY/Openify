# Spotify Connect denemesi (sahibi çalıştırır)

Connect'i (telefondan Openify'a çalma aktarmak, telefondan denetlemek) uygulamaya bağlamadan önce, kaynak okumayla çıkardığımız varsayımların gerçek hesapla doğru olduğunu görmemiz gerek (`PLAN-MOTOR-CONNECT.md` §5, 7.0). Bu deneme bunu yapar. Otomatik testler bunu çalıştırmaz: gerçek hesap ve telefon istiyor.

**Ne yapar:** kayıtlı Openify girişinle Spotify'a bağlanır, hesabında **"Openify (deneme)"** adlı bir Connect aygıtı kaydeder (bitince silinir) ve senden telefonda birkaç şey yapmanı ister. **Ne yapmaz:** kütüphane, takip, çalma listesi, hesap ayarı yazmaz; Openify'ın ayar dosyasına ve önbelleğine dokunmaz; kayıtlı girişi silmez. Ses bilgisayardan çıkar (düşük düzeyde, %25).

## Hazırlık

1. Openify'ı kapat (açık kalırsa da sorun olmaz ama karışıklık olmasın) ve telefonunda Spotify'ı aç (aynı hesap).
2. Derle (bir kez, 5–10 dk):

   ```powershell
   cd C:\Users\furka\Desktop\Projects\Openify
   cargo build --release --features connect-spike --target-dir target/spike
   ```

## Çalıştır

3. Hepsi, sırayla (S1–S6 + sonda S7'yi isteyip istemediğini sorar; S6 varsayılan 30 dk boşta bekler):

   ```powershell
   .\target\spike\release\openify.exe connect-spike
   ```

   Önce kısa bir tur istersen (S6'yı 2 dakikaya indirir):

   ```powershell
   .\target\spike\release\openify.exe connect-spike --idle-min 2
   ```

   Yalnız bazı senaryolar (`--only`, virgülle):

   ```powershell
   .\target\spike\release\openify.exe connect-spike --only S1,S2
   .\target\spike\release\openify.exe connect-spike --only S3,S4
   .\target\spike\release\openify.exe connect-spike --only S6 --idle-min 30
   ```

   Başka bir deneme şarkısı: `--track spotify:track:<ID>`. Yardım: `--help`.

4. Program her adımda ne yapacağını yazar. Sorduğunda **e** (evet), **h** (hayır) ya da Enter (atla) yaz. Ctrl+C: o adımı bitirip özetle çıkar (ikinci Ctrl+C anında çıkar, cihaz listede birkaç dakika kalabilir).

## Adımlar (programın seni yönlendirdiği sırayla)

- **S1, görünürlük:** telefonda Spotify'da cihaz listesini aç (şarkı çubuğundaki cihaz simgesi); "Openify (deneme)" görünmeli.
- **S2, karışmama:** sen bir şey yapma; bilgisayar şarkının son 12 saniyesini çalar. Şarkı bitince hiçbir şey durmamalı / yeniden yüklenmemeli (program kendi söyler).
- **S3, aktarma:** telefonda bir şarkı çal, cihaz listesinden "Openify (deneme)"yi seç. Ses bilgisayardan gelmeli. Sonra telefondan sırayla: duraklat, sürdür, ileri sar, sonraki şarkı, ses düzeyi. Bittiğinde Enter.
- **S4, geri alma:** telefonda cihaz listesinden telefonun kendisini seç. Bilgisayar susmalı.
- **S5, yeniden bağlanma:** program oturumu kapatıp yenisini açar; sen telefonda cihaz listesine bak: "Openify (deneme)" **tam 1 kez** olmalı (hayalet / çift aygıt yok).
- **S6, boşta:** program Spotify'a giden "connect-state" isteklerini sayar; sen telefonda bu cihaza dokunma, dakika say. Sonuç **0** olmalı.
- **S7, ayna (isteğe bağlı):** bilgisayarda yerelde şarkı başlar, program sonra Spirc'e aynı şarkıyı aynı yerden yüklettirir; telefon bunu "çalan cihaz" diye gösteriyor mu?

## Sonuç

5. Program sonunda özet tablosu basar ve bir günlük dosyası yazar:

   ```text
   %TEMP%\openify-connect-spike\connect-spike-<tarih-saat>.log
   ```

   Özeti ve bu dosyayı bana ilet. Günlükte yalnız yol adları ve olaylar var (jeton, hesap adı, sorgu dizgisi yok); cihaz adların (telefon vb.) görünür.

6. Yorumlama (plan): **S2–S6'dan biri KALDI ise Connect v1'den çıkar.** S1 kalırsa Spirc bağlı oturumla kurulamıyor (7.3 yaması P1 yeniden düşünülür). S7 yalnız "ayna" özelliğini (yerelde başlayan çalmanın telefonda görünmesi) belirler: geçmezse v1.x.

Sorun çıkarsa: günlük dosyasının yolunu ve ekrandaki son satırları bana ilet. Cihaz listede takılı kalırsa birkaç dakika içinde düşer; Spotify tarafında kalıcı bir değişiklik yapılmaz.

## Deneme geçtikten sonra: Connect'i uygulamada açmak

Uygulamadaki Connect hazır ama **varsayılan KAPALI** ve "Deneysel" etiketli (`PLAN-MOTOR-CONNECT.md` 7.4 – 7.6). Yukarıdaki deneme (S1–S6, istersen S7) geçmeden açma; geçtiyse:

1. Openify'ı normal aç (deneme derlemesi gerekmez; `connect-spike` özelliği uygulamada yok).
2. **Ayarlar → Spotify Connect** bölümünde anahtarı aç. Açınca Spotify oturumu bir kez, kalıcı aygıt kimliğiyle yeniden açılır (birkaç saniye; çalıyorsa şarkı bu arada kesilmemeli, kesilirse günlüğe bak ve bana ilet) ve Openify telefonun cihaz listesine girer.
3. İstersen **Cihaz adı** düğmesine tıkla ve telefonda görünecek adı değiştir (varsayılan "Openify (BİLGİSAYAR ADI)"). Ad değişince oturum yine yenilenir.
4. Telefonda Spotify → cihaz listesi → o adı seç. Ses bilgisayardan çıkar; telefondan duraklat, sürdür, sar, sonraki, ses düzeyi çalışmalı. Openify'da bu sırada Duraklat / Sonraki / Sar da Spotify'a (telefona) iletilir.
5. Openify'da bir şey çalmaya başlamak (liste çal, şarkıya çift tık, radyo) telefondan çıkar ("kim başlattıysa o sürer"); telefonun cihaz listesinden Openify'ı yeniden seçince tekrar telefon sürer. Telefon cihazı geri alınca (listeden kendini seçince) Openify, aktarmadan önce gösterdiği şarkıyı kaldığı yerde duraklatılmış gösterir.
6. Hesapta başka bir aygıt çalıyor ve Openify'da çalan yoksa, hapın üstünde "<aygıt> cihazında çalıyor" şeridi ve **Buraya aktar** düğmesi çıkar. Başka hiçbir uzaktan komut yok.
7. Kapatmak: anahtarı kapat; aygıt telefonun listesinden birkaç saniyede silinir.

Elle doğrulama (planın §6.3'ü, uygulama içinde, gerçek telefonla): S1 – S6'yı bu kez uygulama üzerinden yinele; üstüne: aktarma ve geri alma, telefondan duraklatınca sesin anında kesilmesi, telefondan sarma / sonraki (eski sesin bitmesini beklememeli), Openify'da yeni çalma başlatınca telefondan çıkması, **Buraya aktar** (bu yol denemede YOK: çalışmazsa not et), aç-kapa-aç ve ad değişiminde telefon listesinde aygıtın **tam bir kez** görünmesi (hayalet yok), uygulamayı kapatınca aygıtın listeden düşmesi. Günlük: `%LOCALAPPDATA%\Openify` altındaki `openify.log`; "Spotify Connect" ve "Spirc" satırları anlatır. Sorunda günlüğü ve ne yaptığını ilet; Connect'i kapatmak yeterli geri alma.
