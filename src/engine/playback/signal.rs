//! Çaldan gelen olaylar (`PlayerSignal`) → durum ve arayüz olayları.

use std::time::Instant;

use super::{Advance, Cx, Driver, Effect, Playback, PlayerSignal};
use crate::{
    engine::{Event, NoticeKind, output},
    t,
};

impl Playback {
    pub fn on_signal(&mut self, cx: &mut Cx<'_>, signal: PlayerSignal) {
        let Some(deck) = &self.deck else { return };
        let out = deck.out.clone();
        // AutoMix: çıkıştan gelen yeni şarkı çözümlemelerini deftere al, önbelleğe yazdır.
        self.collect_analyses(cx);
        // Sürücü geçişleri bant içi gelir (bkz. `connect.rs`).
        match signal {
            PlayerSignal::RemoteAttached => return self.attach_spirc(cx),
            PlayerSignal::RemoteDetached => return self.detach_spirc(cx),
            _ => {}
        }
        // Telefon sürerken (`spirc`): sıra, önyükleme, kurtarma ve kayıt bizim
        // değil; yalnız durum olayları arayüze gider, çıkış telefona uyar.
        let spirc = matches!(self.driver, Driver::Spirc(_));
        // Başka aygıtta çalıyordu ama burada çalma başladı: yerel sürer.
        if matches!(self.driver, Driver::Remote(_))
            && matches!(signal, PlayerSignal::Loading | PlayerSignal::Playing { .. })
        {
            self.leave_remote(cx);
        }
        let went_quiet = matches!(signal, PlayerSignal::Paused { .. });
        let event = match signal {
            PlayerSignal::Loading => {
                if spirc {
                    self.spirc_loading();
                } else {
                    self.ignore_stale_failure = false;
                    self.stall_fired = false;
                    self.load_started = Some(Instant::now());
                }
                Event::Loading
            }
            PlayerSignal::Playing { position_ms } => {
                if spirc {
                    self.spirc_resumed();
                }
                self.failures = 0;
                self.retry_after_reconnect = false;
                self.retried_uri = None;
                self.load_started = None;
                self.ignore_stale_failure = false;
                // Duraklatmadan devam: çaların konumu son ÇÖZÜLEN paket;
                // kuyrukta bekleyen (henüz duyulmamış) ses kadar geriden,
                // duraklatmada gösterilen konumdan sürsün. (Şarkı başında
                // `TrackChanged` `last_position`'ı siler: düzeltme yok.)
                let resumed = matches!(self.last_position, Some((_, _, false)));
                let position_ms = if resumed {
                    let audible = out.audible_position_ms(position_ms);
                    log::debug!("Devam: çalar {position_ms} ms, duyulan {audible} ms");
                    audible
                } else {
                    position_ms
                };
                self.last_position = Some((position_ms, Instant::now(), true));
                Event::Playing { position_ms }
            }
            PlayerSignal::Paused { position_ms } => {
                if spirc {
                    // Telefonun duraklatması: ses kuyruğun boyunu beklemeden susar.
                    self.spirc_silence();
                }
                self.load_started = None;
                self.ignore_stale_failure = false;
                // Duyulan konum: kuyrukta kalan ses kadar geride.
                let raw = position_ms;
                let position_ms = out.audible_position_ms(raw);
                log::debug!(
                    "Duraklatıldı: çalar {raw} ms, duyulan {position_ms} ms (kuyrukta {} kare)",
                    out.queued_frames()
                );
                // Çalar ses aygıtı gittiği / açılamadığı için durduysa söyle
                // (Çal o anki varsayılan aygıtı açar, bkz. `output::Device`).
                let device = out.take_device_notice();
                self.last_position = Some((position_ms, Instant::now(), false));
                // Duraklatma: kaldığı yer hemen yazılır.
                cx.persist();
                if let Some(device) = device {
                    cx.notice(
                        NoticeKind::Problem,
                        match device {
                            output::DeviceNotice::Lost => t!(NoticeDeviceLost),
                            output::DeviceNotice::NoDevice => t!(NoticeNoDevice),
                        },
                    );
                }
                Event::Paused { position_ms }
            }
            PlayerSignal::Position { position_ms } => {
                if let Some((_, _, playing)) = self.last_position {
                    self.last_position = Some((position_ms, Instant::now(), playing));
                }
                cx.persist();
                Event::Position { position_ms }
            }
            PlayerSignal::PreloadDue => {
                if spirc {
                    // Spirc kendi önyüklemesini yapar; biz karışmayız.
                    return;
                }
                // Sıradaki şarkıyı önceden yükle: geçiş boşluksuz olsun. Sıra
                // sonradan değişirse (sıraya ekle, karıştırma…) yenilenir.
                // Şarkının sonu önden ~4 sn çözülsün: sıradakinin başı
                // takılsa da duyulmasın (bkz. `output/mod.rs`, şarkı sonu payı).
                // Geçiş (crossfade) açıksa pay bindirme kadar daha derin olmalı:
                // önce istek, sonra pay.
                self.preview_transition(cx);
                out.set_tail(true);
                self.preload_armed = true;
                self.preload_next(cx.queue);
                return;
            }
            PlayerSignal::EndOfTrack => {
                if spirc {
                    // Sıradakini Spirc yükler (telefonun sırası): sonraki
                    // `Loading` kendiliğinden geçiştir, kuyruk atılmaz.
                    self.spirc_end_of_track();
                    return;
                }
                // Arayüz şarkı değişimini sınır DUYULUNCA görsün (bkz. `handover.rs`).
                self.begin_handover(cx);
                // Kendiliğinden geçiş: kuyruk atılmaz (boşluksuz geçiş).
                if !self.advance(cx, Advance::Auto, false) {
                    cx.emit(Event::Ended);
                    // Biten sırada Çal baştan başlasın.
                    cx.fx.push(Effect::Persist(Some(0)));
                }
                return;
            }
            PlayerSignal::Unavailable { uri } => {
                if spirc {
                    // Atlamayı Spirc yapar; bizim kurtarma sayaçlarımız işlemez.
                    log::debug!("Telefondan çalan şarkı çalınamadı: {uri:?}");
                    return;
                }
                self.on_unavailable(cx, uri);
                return;
            }
            PlayerSignal::TrackChanged(info) => {
                self.last_position = None;
                if let Some(url) = info.cover_url.clone() {
                    cx.fx.push(Effect::FetchCover {
                        uri: info.uri.clone(),
                        url,
                    });
                }
                self.track_info = Some(info.clone());
                cx.persist();
                Event::TrackChanged(info)
            }
            PlayerSignal::RemoteAttached | PlayerSignal::RemoteDetached => return,
        };
        cx.emit(event);
        // Yerelde çalma sustu: başka aygıtta çalan varsa artık "orada çalıyor".
        if went_quiet && !spirc {
            self.reeval_remote(cx);
        }
    }
}
