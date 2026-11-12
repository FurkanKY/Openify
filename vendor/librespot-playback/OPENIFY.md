# librespot-playback (Openify kopyası)

Bu klasör [librespot](https://github.com/librespot-org/librespot) projesinin `librespot-playback` paketinin 0.8.0 sürümüdür (crates.io). Openify `Cargo.toml` içindeki `[patch.crates-io]` ile bu kopyayı kullanır.

## Lisans

MIT. Telif: librespot katkıcıları. Lisans metni `LICENSE` dosyasında.

## Değişiklikler

`src/config.rs` (yeni `config::live` modülü) ve `src/player.rs`. İlk madde (ayarlar) iki okuma noktası; ikinci madde (anahtar hatası) `load_track`'te:

- `PlayerConfig` çalar kurulurken sabitleniyor; ses kalitesi (96 / 160 / 320 kbps) ve ses düzeyi eşitleme ayarlardan değiştirilemiyordu (yeniden başlatma gerekirdi).
- `config::live::set_bitrate` ve `set_normalisation` iki küçük atomik değer tutar. Çalar bir şarkıyı yüklerken (`load_track`) kaliteyi, bir şarkı çalmaya başlarken (`send_playback_start` benzeri bölüm) eşitlemeyi buradan okur. Ayar verilmediyse (`None`) çaların kendi yapılandırması geçerlidir.
- Etkisi sıradaki şarkıdan başlar: çalan (ve önceden yüklenmiş) şarkı kesilmez.
- Ses düzeyi eşitleme DÜZEYİ (Sessiz / Normal / Yüksek, 11 Kasım): `config::live::set_normalisation_pregain_db` bir üçüncü atomik değer tutar (0,01 dB biriminde; `i32::MIN` = verilmedi). `load_track` sonrası eşitleme açıkken `normalisation_pregain_db` olarak, `normalisation` ile aynı noktada (`get_factor`'dan hemen önce) okunur. Spotify'ın şarkı verisi "Normal"e (−14 LUFS) göre ölçülü gelir; Sessiz −23 LUFS için −9 dB, Yüksek −11 LUFS için +3 dB ön kazanç verilir (`Settings::normalise_pregain_db`). Dinamik sınırlayıcı (eşik −2 dBFS) +3 dB'de tepeleri tutar.

İyileştirme asıl projeye de önerilebilir.

### Anahtar alınamayınca anahtarsız devam etme kaldırıldı (7 Kasım)

- Upstream, şarkı anahtarı alınamazsa "anahtarsız çözmeyi dene" diyordu. Spotify dosyaları şifreli: sonuç her zaman çözücü hatası ("end of stream") oluyordu. Önbellekteki dosya da (önbellek ŞİFRELİ dosyayı tutar, yani dosya sağlamdı) "okunamayan dosya" sanılıp siliniyordu; Windows günlüğünde `Error removing file from cache` bu yüzdendi (aynı dosyayı yüklemeye çalışan ikinci yükleme ilkinin sildiği dosyayı silmeye çalışıyordu). Gerçek sebep (ölü oturum) da saklı kalıyordu.
- Şimdi `load_track` anahtar hatasında hemen `None` döner (`PlayerEvent::Unavailable`), önbelleğe ve çözücüye hiç dokunulmaz, ve `config::live::note_key_failure()` bir sayacı artırır. Engine (`src/engine/mod.rs`, `on_unavailable`) sayacın artışına bakıp "sebep oturum" der ve yeniden bağlanır (`src/engine/recovery.rs`).
- Çözücü gerçekten okuyamayan önbellek dosyası (anahtar vardı) için: dosya silinip bir kez yeniden indirilir; ikinci kez de olmazsa vazgeçilir (döngü yok); dosya zaten yoksa (`NotFound`) hata sayılmaz.
