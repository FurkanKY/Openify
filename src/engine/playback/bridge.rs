//! `Playback` ile `Engine` arasındaki köprü: ödünç verme (`Cx`) ve etkilerin
//! sırayla yürütülmesi.

use super::{Cx, Effect, Playback, TransportCmd};
use crate::engine::{Engine, NoticeKind, PlaybackEvent};

impl Engine {
    /// `Playback`i sıra ve şarkı bilgileriyle çalıştırır; yazdığı etkileri,
    /// yazıldıkları sırayla, hemen yürütür (çağrı bitmeden sonraki iş
    /// başlamaz: olay sırası doğrudan çağırmakla aynıdır).
    pub(in crate::engine) fn with_pb<R>(
        &mut self,
        f: impl FnOnce(&mut Playback, &mut Cx<'_>) -> R,
    ) -> R {
        let mut fx = Vec::new();
        let link = self.host.facts();
        let result = f(
            &mut self.pb,
            &mut Cx {
                queue: &mut self.queue,
                meta: &self.meta,
                link,
                fx: &mut fx,
            },
        );
        self.run_effects(fx);
        result
    }

    /// Taşıma komutunu etkin sürücüye verir (bkz. `driver.rs`).
    pub(in crate::engine) fn transport(&mut self, cmd: TransportCmd) {
        self.with_pb(|pb, cx| pb.transport(cx, cmd));
    }

    fn run_effects(&mut self, fx: Vec<Effect>) {
        for effect in fx {
            match effect {
                Effect::Emit(event) => self.emit(event),
                Effect::Notice { kind, message } => self.notice(kind, message),
                Effect::Persist(position) => self.persist_at(position),
                Effect::FetchCover { uri, url } => self.fetch_cover(uri, url),
                Effect::QueueChanged => self.queue_changed(),
                Effect::SaveAnalysis { uri, analysis } => self.save_analysis(uri, analysis),
                Effect::Connect(status) => {
                    let _ = self.playback.try_send(PlaybackEvent::Connect(status));
                }
                Effect::Reconnect {
                    retry_current,
                    skip_if_refused,
                    started,
                } => {
                    if self.reconnect(retry_current) {
                        if let Some(message) = started {
                            self.notice(NoticeKind::Working, message);
                        }
                    } else if skip_if_refused {
                        // Bağlanma başlamadı (geri çekilme / giriş gerekli):
                        // çalınamayan şarkıyı bırak.
                        self.with_pb(|pb, cx| pb.skip_failed(cx));
                    }
                }
            }
        }
    }
}
