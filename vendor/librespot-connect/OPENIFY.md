# librespot-connect (Openify kopyası)

Bu klasör [librespot](https://github.com/librespot-org/librespot) projesinin `librespot-connect` paketinin 0.8.0 sürümüdür (crates.io): Spotify Connect (`Spirc`). Openify `Cargo.toml` içindeki `[patch.crates-io]` ile bu kopyayı kullanır. `LICENSE` ve `README.md` (`lib.rs` onu `include_str!` ile alır) paketle birlikte.

Vendor'lanma nedeni: PLAN-MOTOR-CONNECT.md §1'deki bulgular. Yamasız Spirc, Openify kendi çalarıyla çalarken çalar olaylarına (`EndOfTrack`, `Unavailable`, `TimeToPreloadNextTrack`) tepki verip yerel çalmayı durdurur ya da sıradakini kendisi yükler; oturumu kendi bağlar; dışarıya durum bildirmez.

**Spirc yalnız kullanıcı Ayarlar'dan Spotify Connect'i açınca başlatılır** (`Settings::connect_enabled`, varsayılan KAPALI; 7.4 – 7.6 bağlandı, bkz. ARCHITECTURE.md "Spotify Connect"). Kapalıyken hiç kurulmaz. Ana uygulama `Spirc::with_session`i her takılan oturum için bir kez çağırır ve Spirc'in kendi çalar olaylarından (`SessionConnected` / `SessionDisconnected`, `handle_activate` / `handle_disconnect` yollar) "telefon sürmeye başladı / bıraktı" işaretini alır; bunun için ek yama gerekmedi. `connect-spike` özelliği denemeyi ayrıca koşturur.

## Lisans

MIT. Telif: librespot katkıcıları (Paul Lietar ve diğerleri). Lisans metni `LICENSE` dosyasında.

## Değişiklikler (P1 – P4, `OPENIFY:` işaretli)

`src/spirc.rs` ve `src/model.rs`. `scripts/vendor-diff.ps1 -Crate librespot-connect` farkı listeler. Toplam +335 / −6 satır (planın "~100 satır" tahmininin üstünde: mantık ~120 satır, kalanı belge yorumları ve P3 görünüm türleri); mevcut davranışı (`Spirc::new`) değiştiren tek satır P2'nin kapısıdır.

- **P1, `Spirc::with_session`.** `Spirc::new`in gövdesi ortak `Spirc::build`e taşındı (`new` aynı davranışta: aynı imza, kimlik bilgisiyle bağlanır). `with_session(config, session, player, mixer)` kimlik bilgisi almaz ve `session.connect`i ATLAR: oturum zaten bağlı. Yalnız dealer dinleyicilerini ekler (dealer oturum bağlanırken değil `SpircTask::run` içinde, `dealer().start()` ile başlar; dinleyiciler bağlı oturuma da eklenebilir: kaynakta `DealerManager::add_listen_for` hem başlamış hem başlamamış dealer'ı destekler; **canlı kanıtı denemeyle (S1)**). Dealer kurucusu tek kullanımlık: oturum başına bir Spirc. Üç değer döner: `(Spirc, görev, olay alıcısı)`.
- **P2, çalar olaylarını yok sayma.** `SpircTask::handle_player_event` en başta: Spirc etkin değilse (`!connect_state.is_active()`) ya da ana uygulama çalmayı yürütüyorsa (`host_drives`) hiçbir çalar olayını işlemez. `Spirc::set_host_drives(bool)` Openify tarafından, kendi çalmasını başlatınca çağrılır; Spirc yeniden etkinleşince (`handle_activate`, ör. telefondan aktarma) bayrağı kendisi düşürür (aktarmanın ilk olayları yutulmasın). Etkin değilken olayları işlemek upstream'de de anlamsızdı (durum zaten etkin değilken Spotify'a yazılmaz); asıl düzeltilen, Openify'ın kendi `EndOfTrack`inde Spirc'in `handle_next` / `handle_stop` ile `player.stop()` çağırmasıydı.
- **P3, `SpircEvent`.** `model.rs`: `SpircEvent::{State(HostView), Cluster(ClusterView)}` ve `HostView` / `DeviceView` / `ClusterView` (özet görünümler, protokol türleri sızmaz). `State`: `notify()` (Spotify'a durum gönderilirken) ve etkinlikten çıkarken; `Cluster`: ilk `NEW_DEVICE` yanıtında ve her `hm://connect-state/v1/cluster` güncellemesinde (hesaptaki bütün Connect aygıtları, etkin aygıt, çalan oturum: 7.6 `Driver::Remote`un gözlem kaynağı). Kanal yalnız `with_session` ile açılır (`new`de yok, bellek birikmez). Hiçbir şey Spotify'a yeni gönderilmez; var olan durumun özetidir.
- **P4, `Spirc::add_to_queue(uri)`.** `SpircCommand::AddToQueue`: adresi doğrular (`SpotifyUri::from_uri`) ve `ConnectState::add_to_queue` ile kullanıcı sırasına ekler (Spotify'ın "sıraya ekle" davranışı); yalnız etkinken işler. Telefondan çalan oturumda Openify'ın "sıraya ekle"si buradan geçer.

İyileştirme asıl projeye de önerilebilir (P1, P3, P4 katkıya uygun; P2 `host_drives` Openify'a özgü ama genel bir "gömülü kullanım" seçeneği olarak sunulabilir).

## Bilinen sınırlar

- `with_session` ile kurulan Spirc'in oturumu kapanırsa (`Session::is_invalid`) görev biter; Openify yeni oturum için yeni Spirc kurar (7.4).
- Olay görünümlerinde sıradaki şarkı sayısı `HostView::LIST_MAX` (20) ile kesilir.

## Sınama

Birim sınamalar (`spirc::openify_tests`: küme görünümü, duraklatılmış oturum, boş küme, P2 kapısının doğruluk tablosu) sandığın kendi manifestiyle koşar (çalışma alanı üyesi olmadığı için `cargo test -p librespot-connect` çalışmaz):

```powershell
cargo test --release --lib --no-default-features --features rustls-tls-native-roots --manifest-path vendor/librespot-connect/Cargo.toml --target-dir target/vendor-connect-test
```

12 Kasım'da 6 sınama geçti (4 Openify + 2 upstream `shuffle_vec`). Aynı sınama, ağ olmadan, sandığın `src` + `Cargo.toml` + `README.md` kopyasına ana `Cargo.lock` konup `--offline` verilerek de koşturuldu. `Spirc::with_session`, `add_to_queue`, `State` / `Cluster` bildirimlerinin gerçek davranışı gerçek hesap ister: `openify connect-spike` (CONNECT-DENEME.md).
