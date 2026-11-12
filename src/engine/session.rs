//! Oturum yönetimi: Spotify oturumunun yaşam döngüsü.
//!
//! `SessionHost` bağlantının DURUMUNU tutar (bağlı oturum, bağlantı / giriş
//! durumu, yeniden bağlanma sayaçları, hesap türü denetimi); karar verilen
//! geçişler (`begin_reconnect`, `reconnect_failed`, `premium_step`…) saf
//! yöntemlerdir ve burada sınanır. Ağ ve çalarla ilgili işler (`Engine`in
//! `impl`'i, aynı dosyada): giriş, yeniden bağlanma, sağlık denetimi ve bir
//! oturumun motora TAKILMASI.
//!
//! Bir oturum motora tek yoldan girer: `Engine::install_session(oturum,
//! Install::First | Install::Replace)`. `First`: motorda bağlı oturum yoktu
//! (açılış, giriş, ses çıkışı sonradan gelmiş, hesap sonradan Premium'a
//! dönmüş); çalar ve çıkış kurulur. `Replace`: oturum düşmüştü ya da
//! reddedilmişti; çalar korunur, yeni oturumu alır (`rebind`), eskisi kapanır.
//! Spotify Connect buraya bağlanır (`connect.rs`): her takılan oturum için tek yerde
//! bir Spirc başlatılıp bırakılır.
//!
//! Durumlar ve geçişler `login.rs`'te; kurtarma ilkesi (geri çekilme, hangi
//! hatada ne yapılır) `recovery.rs`'te.

use std::{sync::Arc, time::Instant};

use librespot::{
    core::session::Session,
    playback::{
        config::{Bitrate, PlayerConfig},
        mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
        player::PlayerEventChannel,
    },
};

use super::{
    Deck, Engine, Event, Internal, LibrespotPort, NoticeKind, audio_cache, has_output_device,
    login, new_player,
    playback::{LinkFacts, volume_to_u16},
    recovery,
};
use crate::{auth, t};

/// Bağlandıktan sonra hesap türüne bu kadar sağlık turunda (3 sn) bakılır:
/// Spotify'ın ürün bilgisi paketi geç gelirse "ücretsiz" yine yakalanır.
pub(super) const PREMIUM_CHECKS: u32 = 10;

/// Spotify'a bağlanınca oluşan parçalar.
pub(super) struct Connected {
    pub session: Session,
}

/// Bir oturumun motora nasıl takıldığı (bkz. başlık).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Install {
    /// Motorda bağlı oturum yoktu: çalar ve çıkış kurulur.
    First,
    /// Düşmüş / reddedilmiş oturumun yerine yenisi: çalar korunur.
    Replace,
}

/// `SessionHost::begin_reconnect`in kararı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReconnectStart {
    /// Bir yeniden bağlanma zaten sürüyor: ona katıl.
    Joined,
    /// Başarısız denemelerin geri çekilmesi dolmadı: şimdi deneme.
    Backoff,
    /// Başladı (çağıran bağlanmayı başlatır, "bağlanıyor" der).
    Started,
}

/// Motorun bağlantı durumu. Alanlar `Engine`den taşındı, davranış aynı.
pub(super) struct SessionHost {
    /// Bağlı oturum; `None`: henüz / artık bağlı değil (çalar da yok).
    pub conn: Option<Connected>,
    /// Bağlantı / giriş durumu (bkz. `login.rs`); her değişiklik arayüze olay olur.
    pub link: login::LinkState,
    /// Süren tarayıcı girişinin iptal tutamağı.
    pub login: Option<Arc<tokio::sync::Notify>>,
    /// Bağlandıktan sonra hesap türüne bakmak için kalan sağlık turu sayısı.
    pub premium_checks_left: u32,
    /// Giriş / yeniden bağlanma ile gelen ve çaları kurulması beklenen oturum
    /// (motorda bağlantı yokken); komut döngüsü hemen alıp `First` takar.
    pub new_session: Option<Session>,
    /// Yeniden bağlanma sürüyor mu, en son ne zaman denendi?
    pub reconnecting: bool,
    pub last_reconnect: Option<Instant>,
    /// Art arda başarısız yeniden bağlanma sayısı (geri çekilme uzar) ve
    /// başarısız bağlanmanın ardından yeniden denenmesi bekleniyor mu
    /// (bkz. `recovery.rs`).
    pub reconnect_failures: u32,
    pub reconnect_wanted: bool,
}

impl SessionHost {
    pub fn new() -> Self {
        Self {
            conn: None,
            link: login::LinkState::Connecting,
            login: None,
            premium_checks_left: 0,
            new_session: None,
            reconnecting: false,
            last_reconnect: None,
            reconnect_failures: 0,
            reconnect_wanted: false,
        }
    }

    /// Bağlı oturum (varsa).
    #[cfg(test)]
    pub fn session(&self) -> Option<&Session> {
        self.conn.as_ref().map(|c| &c.session)
    }

    /// `Playback`in kurtarma kararları için bilmesi gerekenler.
    pub fn facts(&self) -> LinkFacts {
        LinkFacts {
            reconnecting: self.reconnecting,
            since_reconnect: self.last_reconnect.map(|t| t.elapsed()),
            reconnect_failures: self.reconnect_failures,
        }
    }

    /// Yeniden bağlanmayı başlatmak serbest mi? (Giriş / Premium gerekmiyor:
    /// çağıran `link.may_reconnect()`i ayrıca bakar.) `Started`ta durum
    /// "bağlanıyor"a geçer.
    pub fn begin_reconnect(&mut self) -> ReconnectStart {
        if self.reconnecting {
            return ReconnectStart::Joined;
        }
        // Başarısız denemelerin ardından Spotify'ı boğma.
        if !recovery::backoff_elapsed(
            self.last_reconnect.map(|t| t.elapsed()),
            self.reconnect_failures,
        ) {
            return ReconnectStart::Backoff;
        }
        self.mark_reconnecting();
        ReconnectStart::Started
    }

    /// Bir (yeniden) bağlanma başladı.
    fn mark_reconnecting(&mut self) {
        self.reconnecting = true;
        self.last_reconnect = Some(Instant::now());
    }

    /// Yeniden bağlanma sonuçlandı (başarı ya da başarısızlık): "sürüyor" biter.
    pub fn reconnect_finished(&mut self) {
        self.reconnecting = false;
    }

    /// Ağ yüzünden başarısız: geri çekilme uzar, süre dolunca `check_health`
    /// yeniden dener.
    pub fn reconnect_failed(&mut self) {
        self.reconnect_failures = self.reconnect_failures.saturating_add(1);
        self.reconnect_wanted = true;
    }

    /// Bağlandı: sayaçlar sıfırlanır.
    pub fn reconnect_succeeded(&mut self) {
        self.reconnect_failures = 0;
        self.reconnect_wanted = false;
    }

    /// Reddedildi / ücretsiz hesap: yeniden denemek boşuna.
    pub fn reconnect_refused(&mut self) {
        self.reconnect_wanted = false;
    }

    /// Önceki bağlanma başarısızdı ve yeni bir deneme bekleniyor mu?
    pub fn reconnect_due(&self) -> bool {
        self.reconnect_wanted && !self.reconnecting
    }

    /// Oturum takıldı: bağlı, hesap türüne bakma turları yeniden sayılır.
    fn connected(&mut self) {
        self.link = login::LinkState::Connected;
        self.premium_checks_left = PREMIUM_CHECKS;
    }

    /// Bir sağlık turunda hesap türü denetimi; `true`: hesap ücretsiz.
    pub fn premium_step(&mut self, kind: auth::AccountKind) -> bool {
        if self.premium_checks_left == 0 {
            return false;
        }
        self.premium_checks_left -= 1;
        match kind {
            auth::AccountKind::Free => return true,
            auth::AccountKind::Premium => self.premium_checks_left = 0,
            auth::AccountKind::Unknown => {}
        }
        false
    }
}

impl Engine {
    // ── Oturumu motora takmak ───────────────────────────────────────────

    /// Bir oturumun motora girdiği TEK yer (bkz. dosya başlığı). `First`te
    /// çaları kurar ve çaların olay kanalını döndürür (komut döngüsü sahibi);
    /// ses çıkışı yoksa `None` (oturum atılır, `Event::NoAudioOutput`).
    /// `Replace`te çalar korunur, `None` döner.
    pub(super) fn install_session(
        &mut self,
        session: Session,
        how: Install,
    ) -> Option<PlayerEventChannel> {
        self.host.connected();
        match how {
            Install::First => self.install_first(session),
            Install::Replace => {
                self.install_replace(session);
                None
            }
        }
    }

    /// Bağlantı kuruldu: çaları oluştur, bekleyen komutları çalıştır,
    /// kütüphaneyi tazele.
    fn install_first(&mut self, session: Session) -> Option<PlayerEventChannel> {
        // Adım 1 kararları: 320 kbps, normalleştirme açık.
        let player_config = PlayerConfig {
            bitrate: Bitrate::Bitrate320,
            normalisation: true,
            ..PlayerConfig::default()
        };
        let mixer = SoftMixer::open(MixerConfig::default()).expect("yazılım ses düzeyi açılamadı");
        mixer.set_volume(volume_to_u16(self.pb.volume));
        if !has_output_device() {
            // Bağlantı sorunu değil: "Çevrimdışı" demek yanlış olurdu.
            self.emit(Event::NoAudioOutput);
            return None;
        }
        // Kendi ses çıkışımız (bkz. `output.rs`): ses düzeyi orada, kuyruğun
        // ardından uygulanır; çalara sabit 1,0 verilir ki seviyeler ses
        // düzeyinden bağımsız ölçülsün (sessize alınca çubuklar düzleşmesin).
        let volume = mixer.get_soft_volume();
        let gain = volume.attenuation_factor() as f32;
        let (player, out) = new_player(
            player_config,
            session.clone(),
            self.levels.clone(),
            gain,
            self.pb.crossfade_ms,
        );
        let player_events = player.get_player_event_channel();

        self.emit(Event::Connected {
            username: session.username(),
        });
        self.host.conn = Some(Connected { session });
        self.pb.deck = Some(Deck::new(
            Box::new(LibrespotPort(player)),
            out,
            mixer,
            volume,
        ));
        self.apply_eq();
        self.apply_karaoke();
        self.apply_automix();

        // Önbellekte eksik olan açık listeleri şimdi tamamla.
        self.complete_opened();
        for command in std::mem::take(&mut self.pending) {
            self.on_command(command);
        }
        self.wire_connected();
        self.refresh_library();
        // Spotify Connect (kapalıysa hiçbir şey yapmaz; varsayılan açık).
        self.connect_on_install();
        Some(player_events)
    }

    /// Düşmüş / reddedilmiş oturumun yerine yenisi: çalar korunur.
    fn install_replace(&mut self, session: Session) {
        if self.host.conn.is_none() {
            // Çalar yok: komut döngüsü `First` takar.
            self.host.new_session = Some(session);
            return;
        }
        // Eski oturumun Spirc'i oturumla birlikte ölür (dealer tek kullanımlık):
        // bırak, sürücü `Local`e döner; yenisi aşağıda kurulur.
        self.connect_stop();
        let Some(conn) = self.host.conn.as_mut() else {
            return;
        };
        log::info!("Spotify'a yeniden bağlanıldı");
        self.pb.rebind(&session);
        let username = session.username();
        // Eski oturum kapanır: onda bekleyen anahtar istekleri hemen düşer
        // (bkz. `vendor/librespot-core/src/audio_key.rs`).
        let old = std::mem::replace(&mut conn.session, session);
        old.shutdown();
        self.emit(Event::Connected { username });
        // Bağlantı yokken bekleyen yazmalar şimdi gitsin.
        self.wire_connected();

        // Açık ama eksik kalan listeleri tamamla, çalınamayan şarkıyı yeniden dene.
        self.complete_opened();
        // Çalınamayan / takılan şarkıyı yeniden yükle ya da önyüklemeyi yenile.
        self.with_pb(|pb, cx| pb.after_reconnect(cx));
        // Yeni oturum için (açıksa) yeni Spirc.
        self.connect_on_install();
    }

    /// Arayüzün açtığı ama eksik kalan listeleri Spotify'dan tamamlar.
    fn complete_opened(&mut self) {
        for id in self.opened.clone() {
            if !self.complete.contains(&id) {
                self.fetch_collection(id);
            }
        }
    }

    // ── Bağlantı sağlığı ────────────────────────────────────────────────

    /// Oturum düştüyse (librespot bağlantı hatasında oturumu geçersiz
    /// sayar) yeniden bağlan.
    pub(super) fn check_health(&mut self) {
        let Some(conn) = &self.host.conn else { return };
        // Ürün bilgisi bağlanırken gelmediyse geç gelen "ücretsiz" yakalanır.
        if self.host.premium_checks_left > 0 {
            let kind = auth::session_account_kind(&conn.session);
            if self.host.premium_step(kind) {
                return self.needs_premium();
            }
        }
        let Some(conn) = &self.host.conn else { return };
        if conn.session.is_invalid() {
            self.reconnect(true);
        } else if self.host.reconnect_due() {
            // Önceki bağlanma başarısızdı: geri çekilme dolunca yeniden dene
            // ("birazdan yeniden denenecek" sözü).
            self.reconnect(false);
        }
    }

    // ── Bağlantı / giriş durumu (bkz. `login.rs`) ───────────────────────

    /// Durumu değiştirir ve arayüze bildirir. `Connected` (kullanıcı adı) ve
    /// `Offline` (hata metni) olaylarını ilgili yerler kendisi yollar.
    pub(super) fn set_link(&mut self, link: login::LinkState) {
        use login::LinkState as S;
        let event = match &link {
            S::Connecting => Some(Event::Connecting),
            S::NeedsLogin => Some(Event::NeedsLogin),
            S::LoggingIn => Some(Event::LoggingIn),
            S::LoginFailed(reason) => Some(Event::LoginFailed(*reason)),
            S::NeedsPremium => Some(Event::NeedsPremium),
            S::Connected | S::Offline => None,
        };
        log::info!("Bağlantı durumu: {link:?}");
        self.host.link = link;
        if let Some(event) = event {
            self.emit(event);
        }
    }

    /// Ağ yok / Spotify'a ulaşılamadı: çevrimdışı. (Açılışta geri çekilmeyle
    /// yeniden denenir.)
    pub(super) fn on_offline(&mut self, e: &anyhow::Error) {
        log::warn!("Spotify'a bağlanılamadı: {e:#}");
        self.host.link = login::LinkState::Offline;
        self.emit(Event::ConnectFailed(format!("{e:#}")));
        // Bağlantıyı bekleyen çalma komutları atılır: internet dakikalar sonra
        // gelince kendiliğinden çalmaya başlamasın.
        self.pending.clear();
        // Önbellek de yoksa kütüphane hiç gelmeyecek: söyle.
        if self.library.is_none() {
            self.emit(Event::LibraryFailed("network: bağlanılamadı".into()));
        }
    }

    /// Kayıtlı giriş yok / reddedildi ya da hesap ücretsiz: kullanıcı eylemi
    /// beklenir (tarayıcı açılmaz, arka planda yeniden denenmez).
    pub(super) fn on_refused(&mut self, refused: auth::ConnectError) {
        match refused {
            auth::ConnectError::NeedsPremium => self.needs_premium(),
            auth::ConnectError::NoCredentials | auth::ConnectError::Rejected => {
                log::info!("Spotify girişi gerekli: {refused}");
                self.pending.clear();
                self.set_link(login::LinkState::NeedsLogin);
                self.library_unavailable();
            }
            // Çağıran ayrı ele alır; yine de güvenli bir karşılık.
            auth::ConnectError::Offline(e) => self.on_offline(&e),
        }
    }

    /// Hesap ücretsiz. Bağlıysak çalar durdurulur ve oturum kapatılır.
    fn needs_premium(&mut self) {
        log::warn!("Hesap Spotify Premium değil: ses akıtılamaz");
        self.pending.clear();
        self.host.premium_checks_left = 0;
        if let Some(conn) = self.host.conn.take() {
            self.connect_stop();
            if let Some(deck) = self.pb.deck.take() {
                deck.stop();
            }
            conn.session.shutdown();
        }
        self.set_link(login::LinkState::NeedsPremium);
        self.library_unavailable();
    }

    /// Giriş / Premium gerekiyor ve önbellekte kütüphane yok: arayüz boş
    /// bir "yükleniyor" yerine nedenini göstersin.
    fn library_unavailable(&self) {
        if self.library.is_none() {
            if let Some(error) = self.host.link.library_error() {
                self.emit(Event::LibraryFailed(error.into()));
            }
        }
    }

    // ── Giriş ───────────────────────────────────────────────────────────

    /// Kullanıcı "Giriş yap"a bastı: tarayıcıda Spotify girişini başlat.
    pub(super) fn start_login(&mut self) {
        if !self.host.link.can_start_login() {
            log::debug!("Giriş başlatılmadı: durum {:?}", self.host.link);
            return;
        }
        let cancel = Arc::new(tokio::sync::Notify::new());
        self.host.login = Some(cancel.clone());
        self.set_link(login::LinkState::LoggingIn);
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let result = login_flow(cancel).await;
            let _ = internal.send(Internal::LoginDone(result));
        });
    }

    /// Kullanıcı girişten vazgeçti: bekleyiş biter, `NeedsLogin`'e dönülür
    /// (sonuç `LoginDone(Err(Cancelled))` ile gelir).
    pub(super) fn cancel_login(&mut self) {
        if self.host.link != login::LinkState::LoggingIn {
            return;
        }
        if let Some(cancel) = &self.host.login {
            cancel.notify_one();
        }
    }

    pub(super) fn on_login_done(&mut self, result: Result<Session, auth::LoginError>) {
        self.host.login = None;
        match result {
            Ok(session) if self.host.conn.is_none() => {
                log::info!("Spotify girişi tamamlandı");
                // Çaları olmayan motor: komut döngüsü `First` takar.
                self.host.new_session = Some(session);
            }
            Ok(session) => {
                log::info!("Spotify girişi tamamlandı");
                // Reddedilmiş eski oturumun yerine yenisi (çalar korunur).
                self.on_reconnected(Ok(session));
            }
            Err(e) => {
                log::warn!("Spotify girişi bitti: {e}");
                match login::after_login_error(&e) {
                    login::LinkState::NeedsPremium => self.needs_premium(),
                    next => {
                        self.set_link(next);
                        self.library_unavailable();
                    }
                }
            }
        }
    }

    // ── Yeniden bağlanma ────────────────────────────────────────────────

    /// Yeniden bağlanmayı başlatır; başlattıysa (ya da sürüyorsa) `true`.
    /// Giriş ya da Premium gerekirken denemez (`LinkState::may_reconnect`).
    pub(super) fn reconnect(&mut self, retry_current: bool) -> bool {
        if !self.host.link.may_reconnect() {
            self.pb.retry_after_reconnect = false;
            return false;
        }
        // Sürene katılan da yeniden yükleme isteyebilir.
        self.pb.retry_after_reconnect |= retry_current;
        match self.host.begin_reconnect() {
            ReconnectStart::Joined => true,
            ReconnectStart::Backoff => {
                // Vazgeçildi: bekleyen yeniden yükleme isteği de düşer.
                if retry_current {
                    self.pb.retry_after_reconnect = false;
                }
                false
            }
            ReconnectStart::Started => {
                log::warn!("Spotify bağlantısı koptu, yeniden bağlanılıyor");
                self.emit(Event::Reconnecting);
                self.spawn_reconnect();
                true
            }
        }
    }

    /// Kayıtlı anahtarla bağlanmayı arka planda dener (tarayıcı AÇMAZ).
    fn spawn_reconnect(&self) {
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let result = first_connect().await;
            let _ = internal.send(Internal::Reconnected(result));
        });
    }

    /// Çalarsız motorda (çevrimdışı, ücretsiz hesap, ses çıkışı yok) kullanıcı
    /// "Yeniden dene"ye bastı: bağlanmayı yeniden dene.
    pub(super) fn retry_connect(&mut self) {
        if self.host.reconnecting {
            return;
        }
        self.host.mark_reconnecting();
        self.set_link(login::LinkState::Connecting);
        self.spawn_reconnect();
    }

    pub(super) fn on_reconnected(&mut self, result: Result<Session, auth::ConnectError>) {
        self.host.reconnect_finished();
        let session = match result {
            Ok(session) => session,
            Err(auth::ConnectError::Offline(e)) if self.host.conn.is_none() => {
                // Çalarsız motorda (retry_connect) geri çekilmeli döngü yok:
                // çevrimdışı göster, kullanıcı yeniden dener.
                self.on_offline(&e);
                return;
            }
            Err(auth::ConnectError::Offline(e)) => {
                log::warn!("Yeniden bağlanılamadı: {e:#}");
                // Geri çekilme dolunca `check_health` yeniden dener.
                self.host.reconnect_failed();
                self.notice(NoticeKind::Working, t!(NoticeReconnectSoon));
                return;
            }
            // Kayıtlı giriş reddedildi / hesap ücretsiz: yeniden denemek boşuna,
            // kullanıcı eylemi gerekir; tarayıcı KENDİLİĞİNDEN açılmaz.
            Err(refused) => {
                self.host.reconnect_refused();
                self.pb.retry_after_reconnect = false;
                self.on_refused(refused);
                return;
            }
        };
        self.host.reconnect_succeeded();
        if self.host.conn.is_none() {
            // Motorda çalar yok (ses çıkışı yoktu ya da hesap ücretsiz çıkmıştı):
            // komut döngüsü çaları kurar (`First`).
            self.host.connected();
            self.host.new_session = Some(session);
            return;
        }
        self.install_session(session, Install::Replace);
    }
}

// ── Bağlanma ve giriş (ağ; geliştirme kancaları) ────────────────────────

/// Açılıştaki (ve yeniden) bağlanma: kayıtlı anahtarla, tarayıcı AÇMADAN.
/// Geliştirme denemeleri (gerçek ağa, Credential Manager'a ve tarayıcıya
/// dokunmaz):
/// - `OPENIFY_TEST_LINK=needs_login|needs_premium`: o duruma düşer (OFFLINE ile birlikte verilirse bu kazanır); `stall`: bağlanma hiç bitmez;
/// - `OPENIFY_TEST_OFFLINE`: ağ hatasıyla düşer (çevrimdışı açılış). Sınama koşusunda
///   bu varsayılandır; gerçek hesaba bağlanmak için `OPENIFY_TEST_ONLINE=1`.
pub(super) async fn first_connect() -> Result<Session, auth::ConnectError> {
    // Geliştirme denemesi: OPENIFY_TEST_PANIC_ENGINE=1 motoru 2 sn sonra çökertir
    // (panik kancası, çökme raporu ve arayüzün "Yeniden başlat" durumu için).
    // Süreçte bir kez: yeniden başlatılan motor normal açılır.
    static PANICKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var_os("OPENIFY_TEST_PANIC_ENGINE").is_some()
        && !PANICKED.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        panic!("OPENIFY_TEST_PANIC_ENGINE: engine crashed on purpose");
    }
    if let Some(link) = std::env::var_os("OPENIFY_TEST_LINK") {
        // `stall`: bağlanma sonsuza dek sürer ("uzun sürüyor" durumunu görmek için).
        if link == "stall" {
            std::future::pending::<()>().await;
        }
        return Err(if link == "needs_premium" {
            auth::ConnectError::NeedsPremium
        } else {
            auth::ConnectError::NoCredentials
        });
    }
    // Sınama koşusu (`OPENIFY_TEST_DISPLAY` / `OPENIFY_SETTINGS_PATH`) açıkça
    // `OPENIFY_TEST_ONLINE=1` verilmedikçe gerçek hesaba bağlanmaz.
    let sandboxed =
        crate::crash::is_test_run() && std::env::var_os("OPENIFY_TEST_ONLINE").is_none();
    if std::env::var_os("OPENIFY_TEST_OFFLINE").is_some() || sandboxed {
        return Err(auth::ConnectError::Offline(anyhow::anyhow!(
            "network: test offline"
        )));
    }
    // Connect AÇIKSA oturum kalıcı aygıt kimliğiyle açılır (telefonun listesinde
    // hep aynı aygıt); kapalıyken bugünkü gibi rastgele kimlik (`connect.rs`).
    match super::connect::session_device_id() {
        Some(id) => auth::connect_saved_with(audio_cache(), Some(&id), true).await,
        None => auth::connect_saved(audio_cache()).await,
    }
}

/// Tarayıcı girişi. Sınama kopyalarında (`OPENIFY_TEST_DISPLAY`,
/// `OPENIFY_TEST_OFFLINE`, `OPENIFY_TEST_LINK`) tarayıcı HİÇ açılmaz: sahte
/// bekleyiş (`OPENIFY_TEST_LOGIN_MS`, varsayılan 4000 ms sonra zaman aşımı;
/// iptal edilebilir).
async fn login_flow(cancel: Arc<tokio::sync::Notify>) -> Result<Session, auth::LoginError> {
    let test = [
        "OPENIFY_TEST_DISPLAY",
        "OPENIFY_TEST_OFFLINE",
        "OPENIFY_TEST_LINK",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some());
    if !test {
        return auth::login(audio_cache(), cancel).await;
    }
    let wait = std::env::var("OPENIFY_TEST_LOGIN_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(4000);
    tokio::select! {
        _ = cancel.notified() => Err(auth::LoginError::Cancelled),
        _ = tokio::time::sleep(std::time::Duration::from_millis(wait)) => Err(auth::LoginError::TimedOut),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn a_new_host_is_not_connected_and_starts_in_connecting() {
        let h = SessionHost::new();
        assert!(
            h.conn.is_none()
                && h.session().is_none()
                && h.new_session.is_none()
                && h.login.is_none()
        );
        assert_eq!(h.link, login::LinkState::Connecting);
        assert!(
            !h.reconnecting
                && !h.reconnect_wanted
                && h.reconnect_failures == 0
                && h.premium_checks_left == 0
        );
        let f = h.facts();
        assert!(!f.reconnecting && f.since_reconnect.is_none() && f.reconnect_failures == 0);
    }

    #[test]
    fn first_reconnect_starts_then_later_callers_join() {
        let mut h = SessionHost::new();
        assert_eq!(h.begin_reconnect(), ReconnectStart::Started);
        assert!(h.reconnecting && h.last_reconnect.is_some());
        assert_eq!(
            h.begin_reconnect(),
            ReconnectStart::Joined,
            "süren bağlanmaya katılınır"
        );
        assert!(h.facts().reconnecting);
    }

    #[test]
    fn a_recent_attempt_holds_the_next_one_back_until_the_backoff_passes() {
        let mut h = SessionHost::new();
        assert_eq!(h.begin_reconnect(), ReconnectStart::Started);
        h.reconnect_finished();
        // Hemen ardından: geri çekilme (10 sn) dolmadı.
        assert_eq!(h.begin_reconnect(), ReconnectStart::Backoff);
        assert!(!h.reconnecting, "Backoff durumu değiştirmez");
        // Süre dolmuş gibi: son deneme 11 sn önce.
        h.last_reconnect = Some(Instant::now() - S(11));
        assert_eq!(h.begin_reconnect(), ReconnectStart::Started);
        // Art arda başarısızlık geri çekilmeyi uzatır: 2 başarısızlıkta 40 sn.
        h.reconnect_finished();
        h.reconnect_failures = 2;
        h.last_reconnect = Some(Instant::now() - S(30));
        assert_eq!(h.begin_reconnect(), ReconnectStart::Backoff);
        h.last_reconnect = Some(Instant::now() - S(41));
        assert_eq!(h.begin_reconnect(), ReconnectStart::Started);
    }

    #[test]
    fn failures_count_up_and_a_success_or_refusal_clears_the_wish_to_retry() {
        let mut h = SessionHost::new();
        h.reconnect_failed();
        h.reconnect_failed();
        assert_eq!(h.reconnect_failures, 2);
        assert!(
            h.reconnect_due(),
            "başarısız bağlanmanın ardından yeniden denenecek"
        );
        h.mark_reconnecting();
        assert!(!h.reconnect_due(), "bağlanma sürerken yeni istek yok");
        h.reconnect_finished();
        assert!(h.reconnect_due());
        h.reconnect_refused();
        assert!(!h.reconnect_due());
        assert_eq!(h.reconnect_failures, 2, "ret sayacı sıfırlamaz");
        h.reconnect_failed();
        h.reconnect_succeeded();
        assert_eq!((h.reconnect_failures, h.reconnect_wanted), (0, false));
    }

    #[test]
    fn connected_marks_the_link_and_rearms_the_account_checks() {
        let mut h = SessionHost::new();
        h.link = login::LinkState::Offline;
        h.connected();
        assert_eq!(h.link, login::LinkState::Connected);
        assert_eq!(h.premium_checks_left, PREMIUM_CHECKS);
    }

    #[test]
    fn account_checks_stop_on_premium_and_flag_a_late_free_account() {
        let mut h = SessionHost::new();
        // Tur kalmadıysa hiçbir şey sorulmaz sayılır.
        assert!(!h.premium_step(auth::AccountKind::Free));
        h.premium_checks_left = 3;
        assert!(!h.premium_step(auth::AccountKind::Unknown));
        assert_eq!(
            h.premium_checks_left, 2,
            "bilinmiyor: tur harcanır, bakmaya devam"
        );
        assert!(
            h.premium_step(auth::AccountKind::Free),
            "geç gelen 'ücretsiz' yakalanır"
        );
        assert!(!h.premium_step(auth::AccountKind::Premium));
        assert_eq!(h.premium_checks_left, 0, "Premium görününce denetim biter");
    }
}
