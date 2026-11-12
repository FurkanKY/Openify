# librespot-core (Openify kopyası)

Bu klasör [librespot](https://github.com/librespot-org/librespot) projesinin `librespot-core` paketinin 0.8.0 sürümüdür (crates.io). Openify `Cargo.toml` içindeki `[patch.crates-io]` ile bu kopyayı kullanır.

## Lisans

MIT. Telif: librespot katkıcıları (Paul Lietar ve diğerleri). Lisans metni `LICENSE` dosyasında.

## Değişiklikler

`src/audio_key.rs` (şarkı anahtarı) ve `src/session.rs` (ücretsiz hesap, en altta):

- Spotify, şarkı anahtarı isteklerini yoğunlukta sınırlıyor: cevabı geciktiriyor (Openify'da ~3,5 sn görüldü) ya da isteği reddediyor (`error audio key 0 2`). Asıl sürüm isteği bir kez gönderip 1,5 sn sonra vazgeçiyor; oynatıcı da anahtarsız devam edip şifreli dosyayı çözemiyor ve şarkı başlamıyor.
- Burada bekleme süresi 4 sn. Zaman aşımında ya da retle karşılaşınca istek 1 sn ve 2 sn aralarla en fazla 3 kez gönderiliyor. Zaman aşımına uğrayan isteğin kaydı siliniyor, böylece geç gelen cevap boşa düşmüyor.
- Alınan anahtar oturum boyunca hatırlanıyor: aynı şarkı yeniden çalınınca Spotify'a tekrar sorulmuyor.

- Bekleme 250 ms'lik adımlarla oturumun geçersiz olup olmadığına bakar (`wait_response`): oturum yeniden bağlanmayla değiştirilip kapatılınca (`Session::shutdown`) ya da bağlantı kapandığı görülünce eski oturumdaki bekleyen anahtar istekleri kalan süreyi beklemeden `SessionClosed` ile düşer; yeni istek de ölü oturuma gönderilmez. (Önceden yeniden bağlandıktan sonra bile eski isteklerin 4 sn × 3 deneme dolması bekleniyordu.)

İyileştirme asıl projeye de önerilebilir.

`src/session.rs` (`check_catalogue`):

- Spotify ürün bilgisinde hesap türü "premium" değilse (ücretsiz: `free` / `open`) librespot `exit(1)` ile bütün süreci kapatıyordu; Openify'da pencere hiçbir şey söylemeden kayboluyordu. Artık yalnız günlüğe yazılıyor, süreç kapanmıyor: karar uygulamada (`src/engine/login.rs` `account_kind`, bağlanınca `NeedsPremium` durumu ve açık bir ileti).

### Güvenlik danışmalarını (RUSTSEC) kapatan bağımlılık güncellemeleri (11 Kasım)

`cargo deny check advisories` üç kaynağı librespot-core'dan gösteriyordu. Hepsi bu kopyada giderildi; ikisi yeni bir paket sürümü, biri API uyarlaması. Ayrıntı:

- **`hyper-proxy2` → `hyper-http-proxy` 1.2** (`Cargo.toml`, `src/http_client.rs`). `hyper-proxy2` 0.1 (terk edilmiş) rustls 0.22 / hyper-rustls 0.26 / webpki 0.102'yi çekiyordu: RUSTSEC-2026-0346 / 0349 (rustls) ve -0049 / -0098 / -0099 / -0104 / -0382 (rustls-webpki), üstelik ağaçta ikinci bir rustls demekti. `hyper-http-proxy` aynı API'li devamı (aynı `Proxy`, `Intercept`, `ProxyConnector`); hyper-rustls 0.27 / tokio-rustls 0.26 / rustls 0.23 kullanıyor, yani ağaçtaki tek rustls. Özellik eşlemesi: `hyper-proxy2/tls` → `hyper-http-proxy/native-tls`, `hyper-proxy2/rustls` → `hyper-http-proxy/rustls-tls-native-roots`, `hyper-proxy2/rustls-webpki` → `hyper-http-proxy/rustls-tls-webpki-roots`.
- **Vekil yokken bağlayıcı `ProxyConnector::unsecured`** (`http_client.rs`). Eskiden vekil yokken "hiçbir şeyi yakalamayan" sahte bir vekil (`Intercept::None`, `0.0.0.0`) kuruluyordu ve `from_proxy` ikinci bir TLS istemci yapılandırması (yerel kökleri bir kez daha yükleyerek) kuruyordu. Şimdi vekil yoksa bağlayıcı hiç vekil taşımıyor (davranış aynı: her istek doğrudan gider); yalnız `SessionConfig::proxy` verilirse `Proxy::new(Intercept::All, ..)` + `from_proxy`. Openify vekil ayarı sunmuyor (`proxy: None`), yani canlı yol hep birinci dal. Yan kazanç: istemci kurulumunda ikinci kök sertifika yüklemesi yok. Kullanılmayan `http::Uri` içe aktarımı kaldırıldı.
- **`quick-xml` 0.38 → 0.41** (`Cargo.toml`, `src/session.rs`). RUSTSEC-2026-0194 / 0195 (>=0.41.0 düzeltildi). Tek API farkı: `BytesText::xml_content` artık XML sürümünü alıyor (`quick_xml::XmlVersion::Implicit1_0`; ürün bilgisi bildirimsiz XML, yani 1.0 varsayılır). Aynı sürüm ağaçtaki öteki quick-xml kullanıcılarıyla (wayland-scanner, xcb; yalnız Linux) aynı olduğundan Cargo.lock'ta tek quick-xml sürümü kalır. ProductInfo ayrıştırması `dispatch` içinden `parse_product_info`'ya (aynı kod) alındı ve premium / ücretsiz hesap örnekleriyle sınanıyor.
- **`rsa` 0.9 → 0.10** (`Cargo.toml`, `src/connection/handshake.rs`). RUSTSEC-2023-0071 (Marvin saldırısı) 0.10.0'da düzeltildi. Bu sandık yalnız **açık anahtarla doğrulama** yapıyor (sunucunun DH anahtarı imzasını sabit Spotify RSA anahtarına karşı denetler: araya girmeye karşı); özel anahtar işlemi (çözme / imzalama) hiçbir yerde yok, yani Marvin yüzeyi zaten yoktu, ama yükseltme danışmayı ignore gerektirmeden kapatıyor. rsa 0.10 `num-bigint` yerine `crypto-bigint` (`BoxedUint`) kullanıyor ve digest 0.11 üzerine kurulu; bu yüzden `Cargo.toml`'a yalnız bu denetim için `sha1_v11` (= `sha1` 0.11, `oid` özelliği) eklendi; HMAC / PBKDF2 / anahtar türetme `sha1` 0.10'da kaldı. rsa `default-features = false, features = ["pkcs1v15"]` (özel anahtar üretimi / PEM / getrandom gerekmiyor). Doğrulama `verify_server_signature`'a taşındı (aynı mantık: RSA-2048, e = 65537, PKCS#1 v1.5, SHA-1) ve OpenSSL ile üretilmiş bilinen-cevap vektörüyle sınanıyor (geçerli imza kabul, bozuk veri / bozuk imza / başka anahtar / boş ve kısa imza ret; sabit Spotify anahtarının geçerli bir RSA açık anahtarı olduğu da denetleniyor).

Bu üç değişiklik oturum açma el sıkışmasını (`handshake.rs`), ürün bilgisinden hesap türünü (`session.rs`) ve HTTP istemcisini (`http_client.rs`) etkiler; hepsinin davranışı korunmuştur, riskler ve ölçümler için yukarıdaki sınama vektörlerine bakın. Canlı Spotify girişi bu değişikliklerle denenmedi.

Not: sandık kendi başına (`cargo test --manifest-path vendor/librespot-core/Cargo.toml --lib --no-default-features --features rustls-tls-native-roots --target-dir target/vendor-core-test`) sınanır; ana çalışma alanının üyesi olmadığı için `cargo test -p librespot-core` çalışmaz.
