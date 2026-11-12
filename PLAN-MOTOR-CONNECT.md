# Plan: çalma motoru (#5), çıkış hattı (#6), oturum + Spotify Connect (#7)

Mimari plan, 11 Kasım 2026. Yalnız kod okunarak yazıldı; Spirc
davranışları librespot-connect 0.8.0 kaynağından, canlı kanıtı denemeye (7.0) bırakıldı.

## 0. Kararlar

1. **Sürücü kuralı "kim başlattıysa o sürer":** Openify'da başlatılan çalma `Driver::Local` (bizim `Queue`, kurallar, dengeli karıştırma); telefondan başlatılan / aktarılan `Driver::Spirc` (gözleriz, komut iletiriz); başka aygıt çalıyorsa `Driver::Remote` (gözlem + "buraya aktar").
2. **`Queue` `Engine`'de kalır**, sürücülere `Cx` ile ödünç verilir; `queue.rs`, `wire.rs`, `edits.rs` hiçbir adımda değişmez.
3. **Crossfade yazar tarafında overlap-add.** Halka tek yazarlı, zaman ekseni doğrusal kalır (konum, söz saati, seviye hesabı bozulmaz). Okur tarafında iki noktadan okuma reddedildi.
4. **Sınır karesini yazar yakalar** (`EndOfTrack`'i çalar iş parçacığında görünce); Spirc ile ilerleme yarışını bu çözer.
5. **Vendor yaması 4 parça, ~100 satır.** Çıkış kapısının işareti `VolumeChanged`'den `SessionClientChanged`'e taşınır.
6. **Bilinen boşluk** (yerelde başlatılan çalma telefonda görünmez): v1'de yalnız deneme S7 geçerse "ayna" (7.8) ile kapanır; yoksa v1.x.

## 1. Planı değiştiren bulgular

- Spirc etkin değilken de çalar olaylarına tepki verir (`handle_player_event` `is_active()` denetlemez): bizim `EndOfTrack`'imizde `handle_next`/`handle_stop` ile `player.stop()` çağırabilir, `TimeToPreloadNextTrack`'te kendi `preload`'unu yollar. Yamasız takılırsa yerel çalmayı durdurur.
- Spirc `emit_volume_changed_event` çağırır; `output.rs` kapısı `VolumeChanged { volume: mark }`'ı işaret sayıyor → telefondan gelen ses düzeyi kapıyı erken açabilir.
- `Spirc::new` oturumu kendi bağlar; dealer dinleyicileri bağlı oturuma da eklenebilir (deneme S1 kanıtlar).
- Dealer kurucusu tek kullanımlık: oturum başına bir Spirc; Connect'i kapat/aç yeni oturum ister.
- `device_id` her oturumda rastgele: kalıcı kimlik gerekli.
- Başka aygıta komut gönderen uç yok; `Remote` v1'de gözlem + aktarma.

## 2. Sıra ve paralellik

```text
A: 7.0 deneme ─────────────────────────────┐ (bağımsız)
B: 5.1 → 5.2 → 5.3a → 5.3b → 5.3c → 5.4 ───┼→ 7.1 → 7.2 → 7.3 → 7.4 → 7.5 → 7.6 → (7.8)
C: 6.0 → 6.1 → 6.2 ──→ 6.3 → 6.4 → 6.5     │   (6.3, 5.3c'den sonra)
                  └──→ 6.6 (EQ; 6.2'den sonra)
```

## 3. #5 `engine/playback/`

Ölçüt: davranış değişmez; mevcut testler geçer; ≥14 yeni senaryo testi; `mod.rs` < 2300 satır; `Engine` alanı < 33.

- **5.1 Port ve sinyal:** `playback/{mod,port,testkit}.rs`. `trait PlayerPort: Send { load, preload, play, pause, stop, seek, mark(u64), rebind(&Session), key_failures() }`; `enum PlayerSignal { Loading, Playing{position_ms}, Paused{position_ms}, Position{position_ms}, PreloadDue, EndOfTrack, Unavailable{uri}, TrackChanged(TrackInfo), RemoteAttached, RemoteDetached }` + `from_librespot`. `LibrespotPort(Arc<Player>)`; `new_player` imzası aynı (cli.rs). `FakePort` çağrıları kaydeder.
- **5.2 `Deck`:** `Connected::{pause, play, seek, load, set_volume}` birebir `Deck { port, out, mixer, volume }`'a taşınır; `Connected { session, deck }`. Testler: pause duck→mark→pause; seek flush→mark→seek; kullanıcı load flush + kuyruk kapanır; otomatik load flush yok.
- **5.3 `Playback`** (üç yeşil alt adım): 5.3a durum/konum; 5.3b yükleme + kurtarma (`on_player_event` gövdesi → `on_signal`, stall, retry, `after_reconnect`); 5.3c `handover.rs` → `playback/handover.rs` (tutma sürerken persist atlanır). `Cx { queue, meta, link: LinkFacts, fx: &mut Vec<Effect> }`; `Effect { Persist, FetchCover, QueueChanged, Reconnect{retry_current, skip_if_refused}, Notice }`; Engine `fx`'i her çağrıdan sonra sırayla boşaltır (olay sırası korunmalı).
- **5.4 `Driver`:** `TransportCmd { Play, Pause, Seek, Next, Previous, JumpTo, Start{uri, position_ms}, SetVolume }`; `enum Driver { Local(Local) }` (Spirc 7.5'te, Remote 7.6'da). Senaryo testleri: kendiliğinden geçiş, önyükleme yenileme, tekrar-tek, sıra sonu, anahtar hatası→reconnect→aynı şarkı, ikinci hata atlar, 4. hata durur, önyükleme hatası çalanı atlatmaz, eski yükleme hatası yok sayılır, geri yüklenen şarkı konumdan, tutmada seek eski şarkıya, stall kenarları.
- **5.5** ARCHITECTURE.md §3, §9, §12.

## 4. #6 çıkış hattı

Ölçütler: altın özet her adımda aynı; `OPENIFY_TEST_TRANSITION_STALL_MS=3000` ile 0 xrun; `fill` EQ kapalı ≤ +%5, EQ açık 10 bant < 3 µs/blok; bellek crossfade kapalı < +0,1 MiB, açık ≤ +6,5 MiB; arayüz geçişi `Transition.start` ±50 ms.

- **6.0 Altın test (önce, bugünkü kodda):** `golden_output_is_bit_stable` — sanal saatli senaryo (çal, rampa, duck/unduck, flush+kapı, xrun, pay+sınır, aygıt değişimi), çıkan f32 bitlerinin FNV özeti sabit.
- **6.1 Böl (saf taşıma):** `output.rs` → `output/{mod.rs (OutputCtl), writer.rs, source.rs, device.rs, watch.rs, tests.rs}`.
- **6.2 `fill` üç aşama:** `plan(now) -> Plan` (saat, kazanç hedefi, kuşak, flush, hold); `pull(&Plan) -> usize` (zarf×kazanç `coef[i]`, halkadan `raw`, xrun, anchor); `shape(n)` (`[EQ]` → `buf = raw*coef`). EQ kapalıyken bit-aynı.
- **6.3 `Transition { start, overlap }`:** tek `AtomicU64` (44 bit start, 20 bit overlap); `begin_transition/end_transition/transition()`; yazar `EndOfTrack`'te `eot_frame = written`; yeni `head` (yeni şarkının yazma ucu). overlap 0 → davranış aynı.
- **6.4 Büyüyebilen halka:** iki yuvalı `RingBuf`, `reserve(frames)` motor iş parçacığında, geçiş flush onayında; ses iş parçacığında ayırma/kilit yok.
- **6.5 Crossfade:** `TransitionSpec { overlap, curve: Linear|EqualPower|Lut }`, `arm_transition`, `spec_for(prev, next, cfg, how)`; tek ilkel `place(.., Append|Add)`; değişmez: yazar yalnız `read + GUARD`'dan sonrasını değiştirir. Yalnız kendiliğinden geçişte; aynı albüm ardışık ve tekrar-tek → 0; kullanıcı geçişi → 0. `settings.crossfade_ms`.
- **6.6 Ekolayzer:** `output/eq.rs`; `EqCtl` atomik katsayı + seqlock; 10 tepe biquad DF2T f32, katsayı 8 blokta yaklaşır, flush'ta sıfır, denormal kesme; otomatik ön kazanç = −max(0, bileşik yanıtın gerçek tepesi) (12 Kasım: yalnız en büyük tek bant komşu eteklerde kırpardı, bkz. `output/eq.rs`); kapalıyken dal çalışmaz. `Command::SetEq { enabled, gains_db: [f32; 10], preamp_db }`. Kulaklık ayarları OPRA'dan çalışırken indirilir (karar, BEKLEYEN-KARARLAR.md).

AutoMix dikişleri: değişken `Transition.start`, geçiş başına `TransitionSpec`, `Curve::Lut`, `place()` + `read+GUARD`, `head`, `tap.rs` yanına `BeatTap`, sonuçlar `library.db`'de.

## 5. #7 oturum + Connect

- **7.0 Deneme** (ayrı kip `openify connect-spike`, `connect-spike` özelliği; varsayılan derlemeyi etkilemez; GERÇEK hesap gerektirir → sahibi çalıştırır): S1 bağlı oturumla `Spirc::with_session`; S2 etkin değilken karışmama; S3 telefondan aktarma; S4 telefon geri alınca; S5 yeniden bağlanmada aynı `device_id`; S6 Spotify'ı yormama (boşta 30 dk'da 0 PUT); S7 ayna (isteğe bağlı). S2–S6'dan biri kalırsa Connect v1'den çıkar.
- **7.1 Kalıcı aygıt kimliği** (`settings.device_id`, `connect_enabled`, `device_name`).
- **7.2 `SessionHost`** (`engine/session.rs`): bağlantı/giriş/yeniden bağlanma alanları ve işlevleri; tek `install_session(session, First|Replace)`.
- **7.3 Vendor librespot-connect + yamalar:** P1 `with_session`; P2 etkin değilken / ana uygulama sürerken çalar olaylarını yok say; P3 `SpircEvent::{State(HostView), Cluster(ClusterView)}`; P4 `AddToQueue`. İşaret `SessionClientChanged("openify-mark", …)`'e taşınır.
- **7.4 Bağlama:** `ConnectPort`, `SpircLink`, `OutMixer`; Spirc görevi `openify-net`'te.
- **7.5 `Driver::Spirc`:** bant içi `RemoteAttached/Detached`; `foreign_flush`; Spirc kolunda çalara komut yok; Pause anında susar; yerelde yeni çalma → Spirc'ten çık, `Local`.
- **7.6 `Driver::Remote`:** "şurada çalıyor" + buraya aktar.
- **7.8 Ayna** (yalnız S7 geçerse).

## 6. Elle doğrulama (gerçek ses; sahibi)

1. 5.4 + 6.2 sonrası: 30 dk çalma (duraklat, sar, sonraki, geçiş, aygıt değişimi, Bluetooth), 0 xrun.
2. 6.5 + 6.6 sonrası: 5 ve 12 sn crossfade, geçiş ortasında duraklat/sar/aygıt; EQ aç-kapa; bellek; boşta CPU.
3. 7.5 sonrası: telefonla S1–S6 uygulama içinde; aktarma ve geri alma; hayalet aygıt yok.

## 7. Arayüz sözleşmesi

- `Command::SetCrossfade(u32 /* ms, 0 kapalı, ≤ 12000 */)`
- `Command::SetEq { enabled, gains_db: [f32; 10], preamp_db }`
- `Command::Connect(ConnectCommand::{SetEnabled(bool), SetName(String), TransferHere})`
- `PlaybackEvent::Connect(ConnectStatus { driver: Local|Spirc|Remote, device_name, remote: Option<RemoteNow> })`
