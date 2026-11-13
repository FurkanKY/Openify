//! Kütüphane yazmaları (Adım 13): beğen / beğenmekten vazgeç, listeye ekle /
//! çıkar, liste oluştur / adlandır / sil, listede sırala, ve hepsinin
//! kalıcı yeniden deneme kuyruğu.
//!
//! Tasarım: arayüz önce kendi görüntüsünü anında günceller (iyimser), sonra
//! işlemi `EditQueue::enqueue` ile SQLite'a yazar; yazma ancak ondan sonra
//! Spotify'a gider. Yani işlem, Spotify'a ulaşmadan uygulama kapansa bile
//! kaybolmaz: bir sonraki açılışta kuyruk kaldığı yerden sürer. Başarısız bir
//! işlem sessizce atılmaz: ya yeniden denenir ya da `failed` durumunda
//! (gerekçesiyle) kullanıcıya gösterilmek üzere bekler.
//!
//! Spotify'a giden yollar (hepsi resmi masaüstü uygulamasının yolu; Web API
//! kapsamı gerekmez, oturumun login5 belirteci yeter):
//!
//!   beğen / vazgeç   POST /collection/v2/write            (collection2v2 WriteRequest)
//!   sanatçıyı takip  aynı istek, küme "artist" (ARTIST_SET); okuma tarafı
//!   et / bırak       /collection/v2/paging küme "artist". CANLI DOĞRULANMADI:
//!                    elle `cargo test --release live_follow -- --ignored`
//!                    (edits/live.rs; kullanıcının onayıyla).
//!   listeye ekle,
//!   çıkar, adlandır,
//!   sırala           POST /playlist/v2/playlist/{id}/changes   (playlist4 ListChanges)
//!   açıklama, ortak  aynı istek, `UPDATE_LIST_ATTRIBUTES` (description / collaborative)
//!   liste oluştur    POST /playlist/v2/playlist                (ListUpdateRequest)
//!                    + kök listeye ekleme: POST /playlist/v2/user/{kullanıcı}/rootlist/changes
//!   liste sil        kök listeden çıkarma (aynı rootlist/changes, Rem işlemi)
//!   klasörler        kök listede `spotify:start-group:<kimlik>:<ad>` /
//!                    `spotify:end-group:<kimlik>` işaretleri (aynı rootlist/changes)
//!
//! CANLI DOĞRULANDI (7 Kasım, kullanıcının onayıyla, hesabında geçici "Openify
//! deneme" listesiyle; `src/engine/edits/live.rs`, elle:
//! `cargo test --release live_write -- --ignored --nocapture`): oluşturma,
//! araya ekleme, taşıma, adlandırma, tek satır çıkarma, beğen / vazgeç, silme
//! ve geri getirme gerçek Spotify'da çalıştı ve hesap sonunda özdeş kaldı.
//! Bulgular:
//!   - Oluşturma isteği (`POST /playlist/v2/playlist`) ad ve şarkıları yok
//!     sayıyor: boş, adsız liste açılıyor. Ad ve şarkılar sonra ayrı bir
//!     değişiklikle yazılıyor (`fill_new_playlist`).
//!   - `Mov.to_index`: çıkarmadan ÖNCEKİ listede, önüne ekleneceği öğenin
//!     indisi (bkz. `MOV_TO_IS_AFTER_REMOVAL`).
//!   - `Rem` anahtarla (`items_as_key`) listedeki BÜTÜN kopyaları siliyor;
//!     tek satır için sıra numarasıyla (`from_index` + `length` 1) çıkarılıyor
//!     (`changes_remove_rows`).

use std::{
    collections::HashSet,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use librespot::{
    core::{Error as SpotifyError, error::ErrorKind, session::Session},
    protocol::playlist4_external::{
        Add, ChangeInfo, Delta, Item, ListAttributeKind, ListAttributes,
        ListAttributesPartialState, ListChanges, ListUpdateRequest, Mov, Op, Rem,
        SelectedListContent, UpdateListAttributes, op::Kind,
    },
};
use protobuf::{CodedOutputStream, Message};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::migrate::{self, Step};

/// Bir işlem bu süre yazılamazsa (hâlâ ağ yok) pes edilir ve kullanıcıya bildirilir.
const MAX_AGE_MS: i64 = 7 * 24 * 3600 * 1000;
const BACKOFF_BASE_MS: i64 = 5_000;
const BACKOFF_MAX_MS: i64 = 15 * 60 * 1000;
/// Sunucu/bilinmeyen hatada en çok bu kadar deneme.
const SERVER_ERROR_ATTEMPTS: u32 = 6;
const CONFLICT_ATTEMPTS: u32 = 4;

const COLLECTION_CONTENT_TYPE: &str = "application/vnd.collection-v2.spotify.proto";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
/// Kök listeye (klasörlerin yaşadığı liste) yazan işlemlerin hedefi: aynı
/// anda tek bir kök liste yazması gitsin diye hepsi bu hedefte sıralanır.
const ROOTLIST_TARGET: &str = "rootlist";
/// Kök listedeki klasör işaretleri.
const FOLDER_START: &str = "spotify:start-group:";
const FOLDER_END: &str = "spotify:end-group:";
/// Liste açıklamasının en çok uzunluğu (Spotify da 300 karakterde kesiyor).
pub const DESCRIPTION_MAX: usize = 300;
/// Koleksiyon servisinin kümeleri: şarkılar ve albümler "collection", takip
/// edilen sanatçılar "artist" (okuma tarafı `library.rs`'te aynı adlarla).
const COLLECTION_SET: &str = "collection";
pub const ARTIST_SET: &str = "artist";

/// `Mov.to_index`, bloğun çıkarılmasından SONRAKİ listedeki yeri mi (true),
/// yoksa çıkarmadan ÖNCEKİ listede, önüne ekleneceği öğenin indisi mi (false)?
/// CANLI DOĞRULANDI (7 Kasım, kullanıcının hesabında deneme listesiyle):
/// ikincisi. [A,D,B,C] listesinde (from 0, length 1, to_index 2) gönderince
/// sonuç [D,A,B,C] oldu: A, çıkarmadan ÖNCEKİ listedeki 2. öğenin (B) önüne
/// girdi. Yani istenen son yer `to` ise teldeki değer ileri taşımada
/// `to + length`, geri taşımada `to`. Bkz. `wire_to_index` ve
/// `mov_wire_semantics_match_live_result` testi.
const MOV_TO_IS_AFTER_REMOVAL: bool = false;

// ── Hata türü ───────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("Çevrimdışı ya da Spotify'a ulaşılamıyor: {0}")]
    Offline(String),
    #[error("Spotify yavaşlamamı istedi")]
    RateLimited { retry_after_ms: Option<u64> },
    #[error("Oturum geçersiz, yeniden giriş gerekiyor: {0}")]
    Unauthorized(String),
    #[error("Bu işleme izin yok (liste senin değil mi?): {0}")]
    Forbidden(String),
    #[error("Bulunamadı: {0}")]
    NotFound(String),
    #[error("Liste bu arada değişti (sürüm çakışması): {0}")]
    Conflict(String),
    #[error("Geçersiz istek: {0}")]
    Invalid(String),
    #[error("Bu işlem Spotify tarafından desteklenmiyor: {0}")]
    Unsupported(String),
    #[error("Spotify hatası: {0}")]
    Server(String),
    /// Yaş sınırı doldu (`MAX_AGE_MS`): işlem pes edildi, kullanıcıya bildirilir.
    #[error("{0}")]
    Expired(String),
    /// Liste oluşturuldu ama kök listeye eklenemedi: tekrar denenirken yeni
    /// liste açılmasın diye oluşan adres saklanır.
    #[error("Liste oluşturuldu ({created_uri}) ama kütüphaneye eklenemedi: {source}")]
    PartialCreate {
        created_uri: String,
        source: Box<EditError>,
    },
}

/// Başarısızlığın türü: arayüz nedeni işleme ve türe göre söylesin diye
/// (ham hata metninden tahmin edilmez). Kuyrukta metin olarak saklanır
/// (`edits.error_kind`), uygulama kapanıp açılınca da kaybolmaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailKind {
    /// Spotify'a ulaşılamıyor.
    Offline,
    /// Spotify yavaşlamamızı istedi.
    RateLimited,
    /// Oturum geçersiz.
    Unauthorized,
    /// Spotify bu işleme izin vermedi (403).
    Forbidden,
    /// Liste / şarkı / albüm Spotify'da yok.
    NotFound,
    /// Liste başka yerde de değişti (sürüm çakışması).
    Conflict,
    /// İstek geçersiz.
    Invalid,
    /// Spotify bu işlemi desteklemiyor.
    Unsupported,
    /// Sunucu hatası (500 ...).
    Server,
    /// Günlerce yazılamadı, pes edildi.
    Expired,
    /// Bağlı olduğu liste oluşturulamadığı için yazılamadı.
    ParentFailed,
    /// Eski sürümden kalan kayıt: tür saklanmamış.
    Unknown,
}

impl FailKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::RateLimited => "rate_limited",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::Server => "server",
            Self::Expired => "expired",
            Self::ParentFailed => "parent_failed",
            Self::Unknown => "unknown",
        }
    }

    fn from_tag(tag: &str) -> Self {
        [
            Self::Offline,
            Self::RateLimited,
            Self::Unauthorized,
            Self::Forbidden,
            Self::NotFound,
            Self::Conflict,
            Self::Invalid,
            Self::Unsupported,
            Self::Server,
            Self::Expired,
            Self::ParentFailed,
        ]
        .into_iter()
        .find(|k| k.tag() == tag)
        .unwrap_or(Self::Unknown)
    }
}

/// librespot'un `HttpClientError::StatusCode` metni: "Response status code: 500 …".
const HTTP_STATUS_MARKER: &str = "Response status code:";

impl EditError {
    /// Hatanın türü (yarım oluşturmada içindeki hatanın türü).
    pub fn kind(&self) -> FailKind {
        match self.inner() {
            Self::Offline(_) => FailKind::Offline,
            Self::RateLimited { .. } => FailKind::RateLimited,
            Self::Unauthorized(_) => FailKind::Unauthorized,
            Self::Forbidden(_) => FailKind::Forbidden,
            Self::NotFound(_) => FailKind::NotFound,
            Self::Conflict(_) => FailKind::Conflict,
            Self::Invalid(_) => FailKind::Invalid,
            Self::Unsupported(_) => FailKind::Unsupported,
            Self::Server(_) => FailKind::Server,
            Self::Expired(_) => FailKind::Expired,
            Self::PartialCreate { .. } => FailKind::Unknown,
        }
    }

    /// Spotify/ağ hatasını sınıflandırır.
    pub fn from_spotify(e: &SpotifyError) -> Self {
        let text = e.to_string();
        match e.kind {
            // librespot 500 / 503 / 421 / 451 durum kodlarını da `Unavailable`
            // yapıyor (http_client.rs). Gerçek bağlantı hatasında (hyper
            // connect) bu metin yok: sunucu yanıt verdiyse `Server` (sınırlı
            // deneme), yoksa çevrimdışı.
            ErrorKind::Unavailable if text.contains(HTTP_STATUS_MARKER) => Self::Server(text),
            ErrorKind::Unavailable
            | ErrorKind::DeadlineExceeded
            | ErrorKind::Aborted
            | ErrorKind::DataLoss
            | ErrorKind::Cancelled => Self::Offline(text),
            ErrorKind::ResourceExhausted => Self::RateLimited {
                retry_after_ms: None,
            },
            ErrorKind::Unauthenticated => Self::Unauthorized(text),
            ErrorKind::PermissionDenied => Self::Forbidden(text),
            ErrorKind::NotFound => Self::NotFound(text),
            // 412 / 428: sürüm uyuşmazlığı.
            ErrorKind::FailedPrecondition => Self::Conflict(text),
            ErrorKind::InvalidArgument | ErrorKind::OutOfRange => Self::Invalid(text),
            ErrorKind::Unimplemented => Self::Unsupported(text),
            // 409 ve tanınmayan her şey buraya düşüyor (http_client.rs).
            _ => Self::Server(text),
        }
    }

    fn inner(&self) -> &EditError {
        match self {
            Self::PartialCreate { source, .. } => source.inner(),
            other => other,
        }
    }

    /// Oluşturulmuş ama tamamlanmamış listenin adresi.
    pub fn partial_create(&self) -> Option<&str> {
        match self {
            Self::PartialCreate { created_uri, .. } => Some(created_uri),
            _ => None,
        }
    }

    /// En çok kaç deneme (None: süresiz; yaş sınırına kadar).
    fn attempt_budget(&self) -> Option<u32> {
        match self.inner() {
            Self::Offline(_) | Self::RateLimited { .. } | Self::Unauthorized(_) => None,
            Self::Server(_) => Some(SERVER_ERROR_ATTEMPTS),
            Self::Conflict(_) => Some(CONFLICT_ATTEMPTS),
            Self::Forbidden(_)
            | Self::NotFound(_)
            | Self::Invalid(_)
            | Self::Unsupported(_)
            | Self::Expired(_)
            | Self::PartialCreate { .. } => Some(0),
        }
    }

    /// `attempts` başarısız denemeden sonra yeniden denenmeli mi?
    pub fn is_retryable(&self, attempts: u32) -> bool {
        self.attempt_budget().is_none_or(|n| attempts < n)
    }

    /// İstek sunucuya ulaşmış (ve belki işlenmiş) olabilir mi? Öyleyse
    /// yeniden denemeden önce durum kontrol edilir (çift eklemeyi önler).
    fn may_have_applied(&self) -> bool {
        matches!(self.inner(), Self::Offline(_) | Self::Server(_))
    }

    /// Bağlantı yok: kuyruğun geri kalanını denemenin anlamı yok.
    fn is_offline(&self) -> bool {
        matches!(self.inner(), Self::Offline(_))
    }
}

impl From<anyhow::Error> for EditError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast_ref::<SpotifyError>() {
            Some(s) => Self::from_spotify(s),
            None => Self::Invalid(format!("{e:#}")),
        }
    }
}

impl From<SpotifyError> for EditError {
    fn from(e: SpotifyError) -> Self {
        Self::from_spotify(&e)
    }
}

/// Yeniden deneme gecikmesi: 5 sn, 15 sn, 45 sn ... en çok 15 dk.
fn backoff_ms(attempts: u32, error: &EditError) -> i64 {
    let exp = BACKOFF_BASE_MS.saturating_mul(3_i64.saturating_pow(attempts.saturating_sub(1)));
    let delay = exp.min(BACKOFF_MAX_MS);
    match error.inner() {
        EditError::RateLimited {
            retry_after_ms: Some(ms),
        } => delay.max(*ms as i64),
        _ => delay,
    }
}

// ── İşlemler ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddPosition {
    First,
    Last,
    /// Verilen sıraya (0 tabanlı) eklenir; ilk eklenen öğe o sırada durur.
    At(u32),
}

/// Kuyruğa yazılan, JSON'a çevrilebilen bir kütüphane işlemi. Hepsi
/// "ne olmalı" biçiminde: yeniden uygulanması zarar vermez ya da yeniden
/// uygulanıp uygulanmadığı denetlenebilir (bkz. `EditBackend`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Şarkıyı (spotify:track:) ya da albümü (spotify:album:) kütüphaneye kaydet.
    Save {
        uri: String,
    },
    Unsave {
        uri: String,
    },
    /// Sanatçıyı (spotify:artist:) takip et / takipten çık: koleksiyon
    /// servisinin `artist` kümesine yazılır (`ARTIST_SET`).
    Follow {
        uri: String,
    },
    Unfollow {
        uri: String,
    },
    AddToPlaylist {
        playlist: String,
        uris: Vec<String>,
        position: AddPosition,
    },
    /// Listeden TEK SATIR çıkarır (karar 7 Kasım): `uris[i]` için `indices[i]`
    /// sırasındaki satır silinir; sıra bilinmiyorsa (`indices` boş ya da kısa)
    /// ya da o sırada başka şarkı varsa o şarkının ilk kopyası silinir. Aynı
    /// adres birden çok kez verilirse o kadar kopya silinir, hiçbir zaman
    /// "bütün kopyalar" değil.
    RemoveFromPlaylist {
        playlist: String,
        uris: Vec<String>,
        #[serde(default)]
        indices: Vec<u32>,
    },
    /// `temp_id` ("local:playlist:...") arayüzün hemen kullanabileceği geçici
    /// adres; liste oluşunca gerçek adrese çevrilir (bkz. `rewrite_target`).
    CreatePlaylist {
        temp_id: String,
        name: String,
        uris: Vec<String>,
        /// Oluşturulmuş ama kök listeye eklenmemişse gerçek adres.
        #[serde(default)]
        created: Option<String>,
    },
    RenamePlaylist {
        playlist: String,
        name: String,
        /// Geri alma için (biliniyorsa).
        #[serde(default)]
        previous_name: Option<String>,
    },
    /// Kök listeden çıkarır (sahibi sensen liste silinmiş olur, değilsen
    /// takipten çıkarsın). Spotify bir süre geri getirmeye izin veriyor.
    DeletePlaylist {
        playlist: String,
    },
    /// `DeletePlaylist`'in geri alınması: kök listeye yeniden ekler.
    RestorePlaylist {
        playlist: String,
    },
    /// `from_index`'ten başlayan `length` öğelik bloğu, sonuçta ilk öğesi
    /// `to_index`'te olacak şekilde taşır. `uris` taşınan öğeler (yeniden
    /// denemede uygulanıp uygulanmadığını anlamak için).
    MovePlaylistItems {
        playlist: String,
        from_index: u32,
        length: u32,
        to_index: u32,
        uris: Vec<String>,
    },
    /// Listenin açıklaması. Boş açıklama alanı siler (kök listede de boş görünür).
    SetPlaylistDescription {
        playlist: String,
        description: String,
        /// Geri alma için (biliniyorsa; boş metin "açıklama yok" demek).
        #[serde(default)]
        previous: Option<String>,
    },
    /// Ortak liste (başkaları da şarkı ekleyebilir) aç / kapa.
    SetPlaylistCollaborative {
        playlist: String,
        collaborative: bool,
        #[serde(default)]
        previous: Option<bool>,
    },
    /// Kök listede boş bir klasör açar: `spotify:start-group:<kimlik>:<ad>` ve
    /// `spotify:end-group:<kimlik>` çifti listenin sonuna eklenir. `key` tam
    /// başlangıç işaretidir (kimlik + yüzde kodlu ad); yeniden denemede aynı
    /// kimlik bulunursa yeniden açılmaz.
    CreateFolder {
        key: String,
        name: String,
    },
    /// Silinen klasörü geri getirir: işaretleri eski yerine koyar ve doğrudan
    /// içindeki listeleri sırayla geri taşır (`DeleteFolder`'ın tersi).
    RestoreFolder {
        folder: String,
        name: String,
        /// Klasörden sonra gelen liste (`None`: kök listenin sonu); yeri
        /// bulmak için çapa.
        #[serde(default)]
        before: Option<String>,
        /// Geri konacak doğrudan içerik (liste adresleri, sırayla).
        #[serde(default)]
        contents: Vec<String>,
    },
    /// Klasörü yeniden adlandırır: ad işaretin kendisinde durduğu için
    /// işaret silinip aynı yere yenisi konur.
    RenameFolder {
        folder: String,
        name: String,
        #[serde(default)]
        previous_name: Option<String>,
    },
    /// Klasörü siler: iki işaret kalkar, içindekiler yerinde kalır (kök
    /// seviyeye çıkarlar; iç içe klasörler de yerinde kalır).
    DeleteFolder {
        folder: String,
        name: String,
        /// Geri getirirken klasörün yerini bulmak için: sonraki liste.
        #[serde(default)]
        before: Option<String>,
        /// Geri getirince klasöre geri taşınacak doğrudan içerik.
        #[serde(default)]
        contents: Vec<String>,
    },
    /// Listeyi bir klasöre taşır, `folder` boşsa köke (kök listenin sonuna)
    /// çıkarır.
    MovePlaylistToFolder {
        playlist: String,
        folder: Option<String>,
        /// Geri alma için eski klasör (`None`: kökteydi).
        #[serde(default)]
        previous: Option<String>,
    },
}

impl EditOp {
    /// Sıralamanın ve engellemenin anahtarı: aynı hedefe yazanlar sırayla,
    /// farklı hedefler birbirini beklemeden gider.
    pub fn target(&self) -> String {
        match self {
            Self::Save { .. } | Self::Unsave { .. } => "collection".into(),
            Self::Follow { .. } | Self::Unfollow { .. } => ARTIST_SET.into(),
            Self::CreatePlaylist { temp_id, .. } => temp_id.clone(),
            Self::AddToPlaylist { playlist, .. }
            | Self::RemoveFromPlaylist { playlist, .. }
            | Self::RenamePlaylist { playlist, .. }
            | Self::DeletePlaylist { playlist }
            | Self::RestorePlaylist { playlist }
            | Self::SetPlaylistDescription { playlist, .. }
            | Self::SetPlaylistCollaborative { playlist, .. }
            | Self::MovePlaylistItems { playlist, .. } => playlist.clone(),
            // Kök liste yazmaları birbirini bekler: sıra önemli (klasör aç,
            // içine taşı gibi).
            Self::CreateFolder { .. }
            | Self::RestoreFolder { .. }
            | Self::RenameFolder { .. }
            | Self::DeleteFolder { .. }
            | Self::MovePlaylistToFolder { .. } => ROOTLIST_TARGET.into(),
        }
    }

    /// Geri alma (Ctrl+Z) işlemi; mümkün değilse `None`.
    /// Oluşturmanın geri alınması gerçek adresi gerektirir
    /// (`EditOutcome::Created`'dan sonra `DeletePlaylist`).
    pub fn inverse(&self) -> Option<EditOp> {
        Some(match self {
            Self::Save { uri } => Self::Unsave { uri: uri.clone() },
            Self::Unsave { uri } => Self::Save { uri: uri.clone() },
            Self::Follow { uri } => Self::Unfollow { uri: uri.clone() },
            Self::Unfollow { uri } => Self::Follow { uri: uri.clone() },
            Self::AddToPlaylist { playlist, uris, .. } => Self::RemoveFromPlaylist {
                playlist: playlist.clone(),
                uris: uris.clone(),
                indices: Vec::new(),
            },
            // Tek satır çıkarıldıysa eski yerine döner; çoklu çıkarmada yerler
            // bilinmiyor: sona eklenir (arayüz yerlerini biliyorsa kendi
            // `AddToPlaylist`'ini kurabilir).
            Self::RemoveFromPlaylist {
                playlist,
                uris,
                indices,
            } => Self::AddToPlaylist {
                playlist: playlist.clone(),
                uris: uris.clone(),
                position: match indices.as_slice() {
                    [at] if uris.len() == 1 => AddPosition::At(*at),
                    _ => AddPosition::Last,
                },
            },
            Self::RenamePlaylist {
                playlist,
                name,
                previous_name,
            } => Self::RenamePlaylist {
                playlist: playlist.clone(),
                name: previous_name.clone()?,
                previous_name: Some(name.clone()),
            },
            Self::DeletePlaylist { playlist } => Self::RestorePlaylist {
                playlist: playlist.clone(),
            },
            Self::RestorePlaylist { playlist } => Self::DeletePlaylist {
                playlist: playlist.clone(),
            },
            Self::MovePlaylistItems {
                playlist,
                from_index,
                length,
                to_index,
                uris,
            } => Self::MovePlaylistItems {
                playlist: playlist.clone(),
                from_index: *to_index,
                length: *length,
                to_index: *from_index,
                uris: uris.clone(),
            },
            Self::SetPlaylistDescription {
                playlist,
                description,
                previous,
            } => Self::SetPlaylistDescription {
                playlist: playlist.clone(),
                description: previous.clone()?,
                previous: Some(description.clone()),
            },
            Self::SetPlaylistCollaborative {
                playlist,
                collaborative,
                previous,
            } => Self::SetPlaylistCollaborative {
                playlist: playlist.clone(),
                collaborative: (*previous)?,
                previous: Some(*collaborative),
            },
            Self::CreateFolder { key, name } => Self::DeleteFolder {
                folder: key.clone(),
                name: name.clone(),
                before: None,
                contents: Vec::new(),
            },
            Self::RestoreFolder {
                folder,
                name,
                before,
                contents,
            } => Self::DeleteFolder {
                folder: folder.clone(),
                name: name.clone(),
                before: before.clone(),
                contents: contents.clone(),
            },
            Self::DeleteFolder {
                folder,
                name,
                before,
                contents,
            } => Self::RestoreFolder {
                folder: folder.clone(),
                name: name.clone(),
                before: before.clone(),
                contents: contents.clone(),
            },
            Self::RenameFolder {
                folder,
                name,
                previous_name,
            } => Self::RenameFolder {
                folder: renamed_folder_key(folder, name)?,
                name: previous_name.clone()?,
                previous_name: Some(name.clone()),
            },
            Self::MovePlaylistToFolder {
                playlist,
                folder,
                previous,
            } => Self::MovePlaylistToFolder {
                playlist: playlist.clone(),
                folder: previous.clone(),
                previous: folder.clone(),
            },
            Self::CreatePlaylist { .. } => return None,
        })
    }

    /// Ön denetim: Spotify'a hiç gitmeden reddedilecek işlemler.
    pub fn validate(&self) -> Result<(), EditError> {
        let bad = |m: &str| Err(EditError::Invalid(m.into()));
        let is_track = |u: &str| u.starts_with("spotify:track:");
        match self {
            Self::Save { uri } | Self::Unsave { uri } => {
                if !is_track(uri) && !uri.starts_with("spotify:album:") {
                    return bad("yalnız şarkı ve albüm kaydedilebilir");
                }
            }
            Self::Follow { uri } | Self::Unfollow { uri } => {
                if !is_artist(uri) {
                    return bad("yalnız sanatçı takip edilebilir");
                }
            }
            Self::AddToPlaylist { uris, .. } | Self::RemoveFromPlaylist { uris, .. } => {
                if uris.is_empty() {
                    return bad("öğe listesi boş");
                }
                if !uris.iter().all(|u| is_track(u)) {
                    return bad("listeye yalnız şarkı eklenir");
                }
            }
            Self::CreatePlaylist { name, uris, .. } => {
                if name.trim().is_empty() {
                    return bad("liste adı boş");
                }
                if !uris.iter().all(|u| is_track(u)) {
                    return bad("listeye yalnız şarkı eklenir");
                }
            }
            Self::RenamePlaylist { name, .. } => {
                if name.trim().is_empty() {
                    return bad("liste adı boş");
                }
            }
            Self::SetPlaylistDescription { description, .. } => {
                if description.chars().count() > DESCRIPTION_MAX {
                    return bad("liste açıklaması çok uzun");
                }
            }
            Self::SetPlaylistCollaborative { .. } => {}
            Self::CreateFolder { key, name } => {
                if folder_marker_id(key).is_none() {
                    return bad("klasör işareti geçersiz");
                }
                if name.trim().is_empty() {
                    return bad("klasör adı boş");
                }
            }
            Self::RestoreFolder { folder, name, .. } => {
                if folder_marker_id(folder).is_none() {
                    return bad("klasör işareti geçersiz");
                }
                if name.trim().is_empty() {
                    return bad("klasör adı boş");
                }
            }
            Self::RenameFolder { folder, name, .. } => {
                if folder_marker_id(folder).is_none() {
                    return bad("klasör işareti geçersiz");
                }
                if name.trim().is_empty() {
                    return bad("klasör adı boş");
                }
            }
            Self::DeleteFolder { folder, .. } => {
                if folder_marker_id(folder).is_none() {
                    return bad("klasör işareti geçersiz");
                }
            }
            Self::MovePlaylistToFolder {
                playlist, folder, ..
            } => {
                if playlist_id(playlist).is_err() {
                    return bad("taşınacak liste geçersiz");
                }
                if let Some(folder) = folder
                    && folder_marker_id(folder).is_none()
                {
                    return bad("hedef klasör geçersiz");
                }
            }
            Self::MovePlaylistItems {
                length,
                from_index,
                to_index,
                ..
            } => {
                if *length == 0 {
                    return bad("taşınacak öğe yok");
                }
                if from_index == to_index {
                    return bad("öğe zaten orada");
                }
            }
            Self::DeletePlaylist { .. } | Self::RestorePlaylist { .. } => {}
        }
        Ok(())
    }

    /// Geçici adres `temp`'i gerçek `real` adresine çevirir.
    fn rewrite_target(&mut self, temp: &str, real: &str) {
        match self {
            Self::AddToPlaylist { playlist, .. }
            | Self::RemoveFromPlaylist { playlist, .. }
            | Self::RenamePlaylist { playlist, .. }
            | Self::DeletePlaylist { playlist }
            | Self::RestorePlaylist { playlist }
            | Self::SetPlaylistDescription { playlist, .. }
            | Self::SetPlaylistCollaborative { playlist, .. }
            | Self::MovePlaylistToFolder { playlist, .. }
            | Self::MovePlaylistItems { playlist, .. }
                if playlist == temp =>
            {
                *playlist = real.to_string();
            }
            _ => {}
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Arayüzün, liste Spotify'da oluşmadan önce kullanabileceği benzersiz adres.
pub fn new_temp_id() -> String {
    format!(
        "local:playlist:{:x}-{:x}",
        now_ms(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// "spotify:artist:<kimlik>" adresi mi (kimlik boş değil, yalnız harf / rakam)?
pub fn is_artist(uri: &str) -> bool {
    uri.strip_prefix("spotify:artist:")
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
}

pub fn is_temp_id(uri: &str) -> bool {
    uri.starts_with("local:playlist:")
}

// ── Klasörler (kök liste) ───────────────────────────────────────────────

static FOLDER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Yeni klasörün kalıcı kimliği: 16 onaltılık harf. Kimliği istemci üretiyor
/// (resmî uygulama da öyle yapıyor); ad işaretin içinde durur.
pub fn new_folder_id() -> String {
    format!(
        "{:012x}{:04x}",
        (now_ms() as u64) & 0x0000_ffff_ffff_ffff,
        FOLDER_COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

/// Klasör adını işaretin içine yazılabilir hâle getirir: harf / rakam ve
/// `-_.~` dışındaki her bayt `%XX` olur (okuma tarafı `library::parse_marker`
/// çözüyor: `%XX` ve `+`).
pub fn encode_folder_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Yeni klasörün başlangıç işareti: `spotify:start-group:<kimlik>:<ad>`.
pub fn folder_start_marker(id: &str, name: &str) -> String {
    format!("{FOLDER_START}{id}:{}", encode_folder_name(name))
}

/// Aynı kimliğin bitiş işareti: `spotify:end-group:<kimlik>`.
pub fn folder_end_marker(id: &str) -> String {
    format!("{FOLDER_END}{id}")
}

/// Klasör işaretinin kimliği (`spotify:start-group:<kimlik>[:<ad>]`).
pub fn folder_marker_id(key: &str) -> Option<&str> {
    key.strip_prefix(FOLDER_START)?
        .split(':')
        .next()
        .filter(|id| !id.is_empty())
}

/// Klasör yeniden adlandırılınca işaretin yeni hâli (kimlik aynı kalır).
pub fn renamed_folder_key(key: &str, name: &str) -> Option<String> {
    Some(folder_start_marker(folder_marker_id(key)?, name))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── Saf yardımcılar: listeyi yerelde güncelleme ve denetim ──────────────

/// `order`'daki [from, from+len) bloğunu, sonuçta ilk öğesi `to` indisinde
/// olacak şekilde taşır (arayüzün anında göstereceği sıra). Sınır dışıysa
/// `false` döner ve listeye dokunmaz.
pub fn apply_move<T: Clone>(order: &mut Vec<T>, from: usize, len: usize, to: usize) -> bool {
    let end = from.saturating_add(len);
    if len == 0 || end > order.len() || to + len > order.len() {
        return false;
    }
    let block: Vec<T> = order.drain(from..end).collect();
    for (i, item) in block.into_iter().enumerate() {
        order.insert(to + i, item);
    }
    true
}

/// Taşıma zaten uygulanmış mı: `to` indisinden başlayan `uris` bloğu listede
/// duruyor mu? (Yeniden denemede çift taşımayı önler.)
pub(crate) fn move_applied(order: &[String], to: usize, uris: &[String]) -> bool {
    !uris.is_empty() && order.get(to..to + uris.len()).is_some_and(|w| w == uris)
}

/// Aynı işlem mi? Kuyruktan dönen kayıt, kuyruğa verilenden yalnız yarım
/// kalmış oluşturmanın gerçek adresiyle (`created`) ayrışabilir.
pub fn same_edit(a: &EditOp, b: &EditOp) -> bool {
    match (a, b) {
        (
            EditOp::CreatePlaylist {
                temp_id: ta,
                name: na,
                uris: ua,
                ..
            },
            EditOp::CreatePlaylist {
                temp_id: tb,
                name: nb,
                uris: ub,
                ..
            },
        ) => ta == tb && na == nb && ua == ub,
        _ => a == b,
    }
}

/// Telde `to_index` olarak gidecek değer.
fn wire_to_index(from: u32, length: u32, to: u32) -> i32 {
    let wire = if MOV_TO_IS_AFTER_REMOVAL || to <= from {
        to
    } else {
        to + length
    };
    wire as i32
}

// ── İstek gövdeleri (saf; testlenir) ────────────────────────────────────

fn item(uri: &str) -> Item {
    Item {
        uri: Some(uri.to_string()),
        ..Default::default()
    }
}

fn changes(base: &[u8], ops: Vec<Op>) -> ListChanges {
    ListChanges {
        base_revision: (!base.is_empty()).then(|| base.to_vec()),
        deltas: vec![Delta {
            ops,
            info: Some(ChangeInfo {
                timestamp: Some(now_ms() / 1000),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        }],
        want_resulting_revisions: Some(true),
        want_sync_result: Some(true),
        ..Default::default()
    }
}

pub fn changes_add(base: &[u8], uris: &[String], position: AddPosition) -> ListChanges {
    let add = Add {
        items: uris.iter().map(|u| item(u)).collect(),
        add_first: (position == AddPosition::First).then_some(true),
        add_last: (position == AddPosition::Last).then_some(true),
        from_index: match position {
            AddPosition::At(i) => Some(i as i32),
            _ => None,
        },
        ..Default::default()
    };
    changes(
        base,
        vec![Op {
            kind: Some(Kind::ADD.into()),
            add: Some(add).into(),
            ..Default::default()
        }],
    )
}

pub fn changes_remove(base: &[u8], uris: &[String]) -> ListChanges {
    let rem = Rem {
        items: uris.iter().map(|u| item(u)).collect(),
        items_as_key: Some(true),
        ..Default::default()
    };
    changes(
        base,
        vec![Op {
            kind: Some(Kind::REM.into()),
            rem: Some(rem).into(),
            ..Default::default()
        }],
    )
}

/// Hangi satırların silineceği: `uris[i]` için `indices[i]` sırasındaki satır
/// (o sırada o şarkı duruyorsa), yoksa o şarkının henüz seçilmemiş ilk kopyası.
/// Listede kalmamış şarkı atlanır. Sonuç büyükten küçüğe sıralı (sırası
/// değişmeden uygulanabilsin diye).
pub fn resolve_rows(
    state: &[String],
    uris: &[String],
    indices: &[u32],
    strict: bool,
) -> Vec<(u32, String)> {
    let mut used: HashSet<usize> = HashSet::new();
    let mut rows = Vec::new();
    for (i, uri) in uris.iter().enumerate() {
        let hinted = indices
            .get(i)
            .map(|&ix| ix as usize)
            .filter(|ix| state.get(*ix) == Some(uri) && !used.contains(ix));
        // `strict` (önceki deneme belirsiz): sırası bilinen satır artık orada
        // değilse çıkarılmış sayılır, başka bir kopya silinmez.
        let found = hinted.or_else(|| {
            if strict && indices.get(i).is_some() {
                return None;
            }
            state
                .iter()
                .enumerate()
                .find(|(ix, u)| *u == uri && !used.contains(ix))
                .map(|(ix, _)| ix)
        });
        if let Some(ix) = found {
            used.insert(ix);
            rows.push((ix as u32, uri.clone()));
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    rows
}

/// Sıra numarasıyla satır silme: her satır için bir `Rem` (from_index + length
/// 1), büyük sıradan küçüğe; böylece tek satır silinir, diğer kopyalar durur.
pub fn changes_remove_rows(base: &[u8], rows: &[(u32, String)]) -> ListChanges {
    changes(
        base,
        rows.iter()
            .map(|(index, uri)| Op {
                kind: Some(Kind::REM.into()),
                rem: Some(Rem {
                    from_index: Some(*index as i32),
                    length: Some(1),
                    items: vec![item(uri)],
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            })
            .collect(),
    )
}

pub fn changes_rename(base: &[u8], name: &str) -> ListChanges {
    let update = UpdateListAttributes {
        new_attributes: Some(ListAttributesPartialState {
            values: Some(ListAttributes {
                name: Some(name.to_string()),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    changes(
        base,
        vec![Op {
            kind: Some(Kind::UPDATE_LIST_ATTRIBUTES.into()),
            update_list_attributes: Some(update).into(),
            ..Default::default()
        }],
    )
}

pub fn changes_move(base: &[u8], from: u32, length: u32, to: u32) -> ListChanges {
    let mov = Mov {
        from_index: Some(from as i32),
        length: Some(length as i32),
        to_index: Some(wire_to_index(from, length, to)),
        ..Default::default()
    };
    changes(
        base,
        vec![Op {
            kind: Some(Kind::MOV.into()),
            mov: Some(mov).into(),
            ..Default::default()
        }],
    )
}

/// Liste açıklamasını yazar; boş açıklama alanı temizler (partial state
/// `no_value` ile "bu alan yok" denir).
pub fn changes_description(base: &[u8], description: &str) -> ListChanges {
    let (values, no_value) = if description.is_empty() {
        (
            ListAttributes::default(),
            vec![ListAttributeKind::LIST_DESCRIPTION.into()],
        )
    } else {
        (
            ListAttributes {
                description: Some(description.to_string()),
                ..Default::default()
            },
            Vec::new(),
        )
    };
    changes_attributes(base, values, no_value)
}

/// Ortak liste bayrağını yazar (`false` de açıkça yazılır: alan "yok" değil,
/// "kapalı" olur).
pub fn changes_collaborative(base: &[u8], collaborative: bool) -> ListChanges {
    changes_attributes(
        base,
        ListAttributes {
            collaborative: Some(collaborative),
            ..Default::default()
        },
        Vec::new(),
    )
}

fn changes_attributes(
    base: &[u8],
    values: ListAttributes,
    no_value: Vec<protobuf::EnumOrUnknown<ListAttributeKind>>,
) -> ListChanges {
    let update = UpdateListAttributes {
        new_attributes: Some(ListAttributesPartialState {
            values: Some(values).into(),
            no_value,
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    changes(
        base,
        vec![Op {
            kind: Some(Kind::UPDATE_LIST_ATTRIBUTES.into()),
            update_list_attributes: Some(update).into(),
            ..Default::default()
        }],
    )
}

/// Kök listeye öğe ekler: `at` verilmişse o sıranın önüne, yoksa sona.
pub fn rootlist_add_items(base: &[u8], items: &[String], at: Option<usize>) -> ListChanges {
    let add = Add {
        items: items.iter().map(|u| item(u)).collect(),
        add_last: at.is_none().then_some(true),
        from_index: at.map(|i| i as i32),
        ..Default::default()
    };
    changes(
        base,
        vec![Op {
            kind: Some(Kind::ADD.into()),
            add: Some(add).into(),
            ..Default::default()
        }],
    )
}

/// Kök listeden tek öğe çıkarır (sıra numarasıyla).
pub fn rootlist_remove_item(base: &[u8], index: usize) -> ListChanges {
    changes(
        base,
        vec![Op {
            kind: Some(Kind::REM.into()),
            rem: Some(Rem {
                from_index: Some(index as i32),
                length: Some(1),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        }],
    )
}

/// Kök listede bir bloğu taşır (`changes_move` ile aynı tel anlamı).
pub fn rootlist_move(base: &[u8], from: usize, length: usize, to: usize) -> ListChanges {
    changes_move(base, from as u32, length as u32, to as u32)
}

// ── Klasör konumları (saf; testlenir) ───────────────────────────────────

/// Klasörün kök listedeki sınırları: (başlangıç işareti, bitiş işareti).
/// Kapanmamış klasörde bitiş = başlangıç (tek işaret).
pub fn folder_bounds(uris: &[String], key: &str) -> Option<(usize, usize)> {
    let start = uris.iter().position(|u| u == key)?;
    let id = folder_marker_id(key)?;
    let end_tag = folder_end_marker(id);
    let end = uris[start + 1..]
        .iter()
        .position(|u| *u == end_tag)
        .map(|i| start + 1 + i)
        .unwrap_or(start);
    Some((start, end))
}

/// `index` sırasındaki öğenin içinde durduğu klasörün anahtarı; kökteyse
/// `None`. Açık klasörler yığınla izlenir (iç içe klasörler).
pub fn parent_folder_at(uris: &[String], index: usize) -> Option<String> {
    let mut open: Vec<(String, String)> = Vec::new(); // (kimlik, anahtar)
    for uri in uris.iter().take(index) {
        match super::library::parse_marker(uri) {
            Some(super::library::Tok::Start { key, id, .. }) => open.push((id, key)),
            Some(super::library::Tok::End { id }) => {
                if let Some(depth) = open.iter().rposition(|(i, _)| *i == id) {
                    open.truncate(depth);
                }
            }
            _ => {}
        }
    }
    open.last().map(|(_, key)| key.clone())
}

/// Bir listeyi klasörün içine sokacak son konum: çıkarmadan sonraki listede,
/// bitiş işaretinin hemen önü. `end` bitiş işaretinin şu anki sırası.
pub fn move_into_folder_target(playlist_index: usize, end: usize) -> usize {
    if playlist_index < end { end - 1 } else { end }
}

// ── Klasör işlemlerinin saf planı (sonuç listesi + tel işlemleri) ────────
//
// Uygulama tarafı (`SpotifyBackend`) bu planları sürümle sarıp gönderir;
// sınamalar planı doğrudan, ağ olmadan çalıştırır.

/// Boş klasör aç (listenin sonuna). Zaten açılmışsa `None`.
pub fn folder_create_ops(uris: &[String], key: &str) -> Option<(Vec<String>, Vec<Op>)> {
    let id = folder_marker_id(key)?;
    // Aynı kimlik zaten duruyorsa klasör açılmış (adı değişmiş olabilir).
    if uris.iter().any(|u| folder_marker_id(u) == Some(id)) {
        return None;
    }
    let end = folder_end_marker(id);
    let mut next = uris.to_vec();
    next.push(key.to_string());
    next.push(end.clone());
    let ops = rootlist_add_items(&[], &[key.to_string(), end], None)
        .deltas
        .remove(0)
        .ops;
    Some((next, ops))
}

/// Klasörün işaretlerini sil (içindekiler yerinde kalır). Yoksa `None`.
pub fn folder_delete_ops(uris: &[String], folder: &str) -> Option<(Vec<String>, Vec<Op>)> {
    let (start, end) = folder_bounds(uris, folder)?;
    let mut next = uris.to_vec();
    let mut ops = Vec::new();
    if end > start {
        ops.extend(rootlist_remove_item(&[], end).deltas.remove(0).ops);
        next.remove(end);
    }
    ops.extend(rootlist_remove_item(&[], start).deltas.remove(0).ops);
    next.remove(start);
    Some((next, ops))
}

/// Klasörü yeniden adlandır: işaret aynı yerde yenisiyle değişir. Zaten yeni
/// addaysa `None`; klasör yoksa `NotFound`.
pub fn folder_rename_ops(
    uris: &[String],
    folder: &str,
    name: &str,
) -> Result<Option<(Vec<String>, Vec<Op>)>, EditError> {
    let Some(new_key) = renamed_folder_key(folder, name) else {
        return Err(EditError::Invalid("klasör işareti geçersiz".into()));
    };
    let Some((start, _)) = folder_bounds(uris, folder) else {
        if uris.iter().any(|u| u == &new_key) {
            return Ok(None);
        }
        return Err(EditError::NotFound("klasör kök listede yok".into()));
    };
    if uris[start] == new_key {
        return Ok(None);
    }
    let mut next = uris.to_vec();
    next[start] = new_key.clone();
    let mut ops = rootlist_remove_item(&[], start).deltas.remove(0).ops;
    ops.extend(
        rootlist_add_items(&[], std::slice::from_ref(&new_key), Some(start))
            .deltas
            .remove(0)
            .ops,
    );
    Ok(Some((next, ops)))
}

/// Silinmiş klasörü işaretleriyle ve doğrudan içeriğiyle geri getir.
pub fn folder_restore_ops(
    uris: &[String],
    folder: &str,
    before: Option<&str>,
    contents: &[String],
) -> Option<(Vec<String>, Vec<Op>)> {
    let id = folder_marker_id(folder)?;
    if uris.iter().any(|u| folder_marker_id(u) == Some(id)) {
        return None;
    }
    let end = folder_end_marker(id);
    let at = before.and_then(|b| uris.iter().position(|u| u == b));
    let i = at.unwrap_or(uris.len());
    let mut next = uris.to_vec();
    next.insert(i, folder.to_string());
    next.insert(i + 1, end.clone());
    let mut ops = rootlist_add_items(
        &[],
        &[folder.to_string(), end.clone()],
        (i < uris.len()).then_some(i),
    )
    .deltas
    .remove(0)
    .ops;
    for child in contents {
        let Some(j) = next.iter().position(|u| u == child) else {
            continue;
        };
        let Some(e) = next.iter().position(|u| u == &end) else {
            break;
        };
        let to = move_into_folder_target(j, e);
        if j == to {
            continue;
        }
        ops.extend(rootlist_move(&[], j, 1, to).deltas.remove(0).ops);
        apply_move(&mut next, j, 1, to);
    }
    Some((next, ops))
}

/// Listeyi klasöre (`folder` boşsa köke) taşı. Zaten oradaysa `None`.
pub fn playlist_move_ops(
    uris: &[String],
    playlist: &str,
    folder: Option<&str>,
) -> Result<Option<(Vec<String>, Vec<Op>)>, EditError> {
    let Some(p) = uris.iter().position(|u| u == playlist) else {
        return Err(EditError::NotFound(format!(
            "liste kök listede yok: {playlist}"
        )));
    };
    if parent_folder_at(uris, p).as_deref() == folder {
        return Ok(None);
    }
    let to = match folder {
        Some(key) => {
            let Some((start, end)) = folder_bounds(uris, key) else {
                return Err(EditError::NotFound(format!(
                    "klasör kök listede yok: {key}"
                )));
            };
            if end == start {
                return Err(EditError::Invalid("klasör kapanmamış".into()));
            }
            move_into_folder_target(p, end)
        }
        // Kök: listenin sonu (bütün klasörlerin dışı).
        None => uris.len().saturating_sub(1),
    };
    if to == p {
        return Ok(None);
    }
    let mut next = uris.to_vec();
    if !apply_move(&mut next, p, 1, to) {
        return Err(EditError::Invalid("taşıma sınır dışı".into()));
    }
    let ops = rootlist_move(&[], p, 1, to).deltas.remove(0).ops;
    Ok(Some((next, ops)))
}

/// Yeni liste isteği (adlı, isteğe bağlı başlangıç şarkılarıyla).
pub fn create_request(name: &str, uris: &[String]) -> ListUpdateRequest {
    ListUpdateRequest {
        attributes: Some(ListAttributes {
            name: Some(name.to_string()),
            ..Default::default()
        })
        .into(),
        items: uris.iter().map(|u| item(u)).collect(),
        info: Some(ChangeInfo {
            timestamp: Some(now_ms() / 1000),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    }
}

/// Kök listeye (kütüphanenin liste sırası) bir liste ekler / çıkarır.
pub fn rootlist_add(base: &[u8], playlist: &str) -> ListChanges {
    changes_add(base, &[playlist.to_string()], AddPosition::First)
}

pub fn rootlist_remove(base: &[u8], playlist: &str) -> ListChanges {
    changes_remove(base, &[playlist.to_string()])
}

/// collection2v2.WriteRequest { username = 1; set = 2; repeated CollectionItem items = 3;
/// client_update_id = 4 }, CollectionItem { uri = 1; added_at = 2; is_removed = 3 }.
/// Okuma tarafındaki `set` adı "collection" (şarkılar ve albümler birlikte);
/// takip edilen sanatçılar "artist" kümesinde (`collection_write_body_in`).
/// Üretimde küme her zaman açıkça verilir; bu kısa yol yalnız testlerde.
#[cfg(test)]
pub fn collection_write_body(
    username: &str,
    uris: &[String],
    removed: bool,
    now_secs: i32,
    update_id: &str,
) -> Result<Vec<u8>> {
    collection_write_body_in(COLLECTION_SET, username, uris, removed, now_secs, update_id)
}

/// `collection_write_body`, küme adı verilerek.
pub fn collection_write_body_in(
    set: &str,
    username: &str,
    uris: &[String],
    removed: bool,
    now_secs: i32,
    update_id: &str,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut body);
        os.write_string(1, username)?;
        os.write_string(2, set)?;
        for uri in uris {
            let mut inner = Vec::new();
            {
                let mut is = CodedOutputStream::vec(&mut inner);
                is.write_string(1, uri)?;
                is.write_int32(2, now_secs)?;
                if removed {
                    is.write_bool(3, true)?;
                }
                is.flush()?;
            }
            os.write_bytes(3, &inner)?;
        }
        os.write_string(4, update_id)?;
        os.flush()?;
    }
    Ok(body)
}

/// "spotify:playlist:<22 harf>" → base62 kimlik.
fn playlist_id(uri: &str) -> Result<&str, EditError> {
    let id = uri
        .strip_prefix(PLAYLIST_PREFIX)
        .ok_or_else(|| EditError::Invalid(format!("liste adresi değil: {uri}")))?;
    if id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(id)
    } else {
        Err(EditError::Invalid(format!("geçersiz liste kimliği: {uri}")))
    }
}

// ── Spotify arka ucu ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    Done,
    /// `CreatePlaylist` tamamlandı: gerçek adres.
    Created {
        uri: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ApplyCtx {
    /// Kuyruktaki işlem numarası (istemci güncelleme kimliği olarak da gider).
    pub id: i64,
    /// Önceki deneme sunucuya ulaşmış olabilir: uygulamadan önce durumu denetle.
    pub maybe_applied: bool,
}

/// Bir işlemi gerçekten uygulayan taraf. Gerçeği `SpotifyBackend`; testler
/// sahtesini kullanır.
pub trait EditBackend: Sync {
    fn apply(
        &self,
        op: &EditOp,
        ctx: ApplyCtx,
    ) -> impl Future<Output = Result<EditOutcome, EditError>> + Send;
}

pub struct SpotifyBackend<'a> {
    pub session: &'a Session,
}

impl EditBackend for SpotifyBackend<'_> {
    async fn apply(&self, op: &EditOp, ctx: ApplyCtx) -> Result<EditOutcome, EditError> {
        op.validate()?;
        let s = self.session;
        match op {
            EditOp::Save { uri } => write_collection(s, COLLECTION_SET, uri, false, ctx)
                .await
                .map(|()| EditOutcome::Done),
            EditOp::Unsave { uri } => write_collection(s, COLLECTION_SET, uri, true, ctx)
                .await
                .map(|()| EditOutcome::Done),
            // Takip: aynı servis, "artist" kümesi. Yazma "olması gereken hâli"
            // söyler (idempotent): belirsiz bir önceki deneme yeniden gönderilince
            // zarar vermez.
            EditOp::Follow { uri } => write_collection(s, ARTIST_SET, uri, false, ctx)
                .await
                .map(|()| EditOutcome::Done),
            EditOp::Unfollow { uri } => write_collection(s, ARTIST_SET, uri, true, ctx)
                .await
                .map(|()| EditOutcome::Done),
            EditOp::AddToPlaylist {
                playlist,
                uris,
                position,
            } => {
                // Belirsiz önceki deneme: listede zaten olanları yeniden ekleme.
                let uris = if ctx.maybe_applied {
                    let present = playlist_state(s, playlist).await?;
                    let present: HashSet<&str> = present.uris.iter().map(String::as_str).collect();
                    let rest: Vec<String> = uris
                        .iter()
                        .filter(|u| !present.contains(u.as_str()))
                        .cloned()
                        .collect();
                    if rest.is_empty() {
                        return Ok(EditOutcome::Done);
                    }
                    rest
                } else {
                    uris.clone()
                };
                post_playlist_changes(s, playlist, |st| {
                    Some(changes_add(&st.revision, &uris, *position))
                })
                .await
            }
            EditOp::RemoveFromPlaylist {
                playlist,
                uris,
                indices,
            } => {
                // Tek satır: sıra numarasıyla (aynı şarkının öbür kopyaları kalır).
                // Belirsiz önceki denemede listede artık olmayan şarkı çıkarılmış sayılır.
                post_playlist_changes(s, playlist, |st| {
                    let rows = resolve_rows(&st.uris, uris, indices, ctx.maybe_applied);
                    (!rows.is_empty()).then(|| changes_remove_rows(&st.revision, &rows))
                })
                .await
            }
            EditOp::RenamePlaylist { playlist, name, .. } => {
                post_playlist_changes(s, playlist, |st| Some(changes_rename(&st.revision, name)))
                    .await
            }
            EditOp::MovePlaylistItems {
                playlist,
                from_index,
                length,
                to_index,
                uris,
            } => {
                if ctx.maybe_applied {
                    let state = playlist_state(s, playlist).await?;
                    if move_applied(&state.uris, *to_index as usize, uris) {
                        return Ok(EditOutcome::Done);
                    }
                }
                post_playlist_changes(s, playlist, |st| {
                    Some(changes_move(&st.revision, *from_index, *length, *to_index))
                })
                .await
            }
            EditOp::CreatePlaylist {
                name,
                uris,
                created,
                ..
            } => {
                let uri = match created {
                    Some(uri) => uri.clone(),
                    None => create_playlist(s, name, uris).await?,
                };
                // Spotify, oluşturma isteğindeki adı ve şarkıları yok sayıyor (canlı
                // doğrulandı): boş liste açılır, ad ve şarkılar ayrı bir değişiklikle
                // yazılır. Yeniden denemede zaten yazılmış olan atlanır.
                if let Err(e) = fill_new_playlist(s, &uri, name, uris).await {
                    return Err(EditError::PartialCreate {
                        created_uri: uri,
                        source: Box::new(e),
                    });
                }
                // Kök listeye ekleme: başarısızsa oluşan adres saklansın.
                match post_rootlist_changes(s, |rev| rootlist_add(rev, &uri)).await {
                    Ok(()) => Ok(EditOutcome::Created { uri }),
                    Err(e) => Err(EditError::PartialCreate {
                        created_uri: uri,
                        source: Box::new(e),
                    }),
                }
            }
            EditOp::DeletePlaylist { playlist } => {
                match post_rootlist_changes(s, |rev| rootlist_remove(rev, playlist)).await {
                    // Zaten kütüphanede değil: silinmiş sayılır (idempotent).
                    Err(EditError::NotFound(_)) => Ok(EditOutcome::Done),
                    other => other.map(|()| EditOutcome::Done),
                }
            }
            EditOp::RestorePlaylist { playlist } => {
                post_rootlist_changes(s, |rev| rootlist_add(rev, playlist))
                    .await
                    .map(|()| EditOutcome::Done)
            }
            EditOp::SetPlaylistDescription {
                playlist,
                description,
                ..
            } => {
                // Zaten istenen açıklama yazılıysa istek atılmaz (belirsiz bir
                // önceki denemenin yanıtı kaybolsa da zarar vermez).
                post_playlist_changes(s, playlist, |st| {
                    (st.description != *description)
                        .then(|| changes_description(&st.revision, description))
                })
                .await
            }
            EditOp::SetPlaylistCollaborative {
                playlist,
                collaborative,
                ..
            } => {
                post_playlist_changes(s, playlist, |st| {
                    (st.collaborative != *collaborative)
                        .then(|| changes_collaborative(&st.revision, *collaborative))
                })
                .await
            }
            EditOp::CreateFolder { key, .. } => post_rootlist_plan(s, |st| {
                Ok(folder_create_ops(&st.uris, key).map(|(_, ops)| changes(&st.revision, ops)))
            })
            .await
            .map(|()| EditOutcome::Done),
            EditOp::RestoreFolder {
                folder,
                before,
                contents,
                ..
            } => post_rootlist_plan(s, |st| {
                Ok(
                    folder_restore_ops(&st.uris, folder, before.as_deref(), contents)
                        .map(|(_, ops)| changes(&st.revision, ops)),
                )
            })
            .await
            .map(|()| EditOutcome::Done),
            EditOp::RenameFolder { folder, name, .. } => post_rootlist_plan(s, |st| {
                Ok(folder_rename_ops(&st.uris, folder, name)?
                    .map(|(_, ops)| changes(&st.revision, ops)))
            })
            .await
            .map(|()| EditOutcome::Done),
            EditOp::DeleteFolder { folder, .. } => post_rootlist_plan(s, |st| {
                Ok(folder_delete_ops(&st.uris, folder).map(|(_, ops)| changes(&st.revision, ops)))
            })
            .await
            .map(|()| EditOutcome::Done),
            EditOp::MovePlaylistToFolder {
                playlist, folder, ..
            } => post_rootlist_plan(s, |st| {
                Ok(playlist_move_ops(&st.uris, playlist, folder.as_deref())?
                    .map(|(_, ops)| changes(&st.revision, ops)))
            })
            .await
            .map(|()| EditOutcome::Done),
        }
    }
}

/// Spotify'a hemen (kuyruksuz) tek bir işlem uygular. Arayüz iyimser
/// güncellemeyi `EditQueue::enqueue` ile yapar; bu yalnız kuyruk dışı
/// kullanım (örn. sınama) içindir.
#[cfg_attr(not(test), allow(dead_code))] // canlı sınama (edits/live.rs)
pub async fn apply_now(session: &Session, op: &EditOp) -> Result<EditOutcome, EditError> {
    SpotifyBackend { session }
        .apply(
            op,
            ApplyCtx {
                id: 0,
                maybe_applied: false,
            },
        )
        .await
}

async fn write_collection(
    session: &Session,
    set: &str,
    uri: &str,
    removed: bool,
    ctx: ApplyCtx,
) -> Result<(), EditError> {
    let body = collection_write_body_in(
        set,
        &session.username(),
        &[uri.to_string()],
        removed,
        (now_ms() / 1000) as i32,
        &format!("openify-{}-{}", ctx.id, now_ms()),
    )?;
    let mut headers = http::HeaderMap::new();
    let ct: http::HeaderValue = COLLECTION_CONTENT_TYPE
        .parse()
        .map_err(|e| EditError::Invalid(format!("başlık: {e}")))?;
    headers.insert(http::header::CONTENT_TYPE, ct.clone());
    headers.insert(http::header::ACCEPT, ct);
    session
        .spclient()
        .request(
            &http::Method::POST,
            "/collection/v2/write",
            Some(headers),
            Some(&body),
        )
        .await?;
    Ok(())
}

/// Bir listenin sunucudaki hali: sürüm, şarkı sırası ve yazma işlemlerinin
/// denetlediği öznitelikler.
pub(crate) struct PlaylistState {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) collaborative: bool,
    pub(crate) revision: Vec<u8>,
    pub(crate) uris: Vec<String>,
}

pub(crate) async fn playlist_state(
    session: &Session,
    playlist: &str,
) -> Result<PlaylistState, EditError> {
    let id = playlist_id(playlist)?;
    let endpoint = format!("/playlist/v2/playlist/{id}");
    let bytes = session
        .spclient()
        .request(&http::Method::GET, &endpoint, None, None)
        .await?;
    let list = SelectedListContent::parse_from_bytes(&bytes)
        .map_err(|e| EditError::Server(format!("liste yanıtı çözülemedi: {e}")))?;
    Ok(PlaylistState {
        name: list.attributes.name().to_string(),
        description: list.attributes.description().to_string(),
        collaborative: list.attributes.collaborative(),
        revision: list.revision().to_vec(),
        uris: list
            .contents
            .items
            .iter()
            .map(|i| i.uri().to_string())
            .collect(),
    })
}

/// Kök listenin (kütüphane sırası) ham hâli: sürüm ve öğe adresleri (klasör
/// işaretleri dahil).
pub(crate) struct RootState {
    pub(crate) revision: Vec<u8>,
    pub(crate) uris: Vec<String>,
}

pub(crate) async fn rootlist_state(session: &Session) -> Result<RootState, EditError> {
    let bytes = session.spclient().get_rootlist(0, Some(1000)).await?;
    let list = SelectedListContent::parse_from_bytes(&bytes)
        .map_err(|e| EditError::Server(format!("kök liste çözülemedi: {e}")))?;
    Ok(RootState {
        revision: list.revision().to_vec(),
        uris: list
            .contents
            .items
            .iter()
            .map(|i| i.uri().to_string())
            .collect(),
    })
}

/// `build(sürüm)` ile kurulan değişikliği listeye gönderir. Sürüm çakışırsa
/// (başka cihazda aynı anda değişiklik) listeyi yeniden okuyup bir kez daha dener;
/// Spotify değişiklikleri sürüm farkına göre kendisi birleştirir, yani veri
/// kaybedilmez.
async fn post_playlist_changes(
    session: &Session,
    playlist: &str,
    build: impl Fn(&PlaylistState) -> Option<ListChanges>,
) -> Result<EditOutcome, EditError> {
    let id = playlist_id(playlist)?;
    let endpoint = format!("/playlist/v2/playlist/{id}/changes");
    let mut last = None;
    for _ in 0..2 {
        let state = playlist_state(session, playlist).await?;
        // None: yapılacak bir şey kalmamış (örn. çıkarılacak şarkı zaten yok).
        let Some(body) = build(&state) else {
            return Ok(EditOutcome::Done);
        };
        match session
            .spclient()
            .request_with_protobuf(&http::Method::POST, &endpoint, None, &body)
            .await
        {
            Ok(_) => return Ok(EditOutcome::Done),
            Err(e) => match EditError::from_spotify(&e) {
                e @ EditError::Conflict(_) => last = Some(e),
                e => return Err(e),
            },
        }
    }
    Err(last.unwrap_or_else(|| EditError::Conflict("sürüm çakışması".into())))
}

async fn post_rootlist_changes(
    session: &Session,
    build: impl Fn(&[u8]) -> ListChanges,
) -> Result<(), EditError> {
    let user = session.username();
    let list_endpoint =
        format!("/playlist/v2/user/{user}/rootlist?decorate=revision&from=0&length=1");
    let endpoint = format!("/playlist/v2/user/{user}/rootlist/changes");
    let mut last = None;
    for _ in 0..2 {
        let bytes = session
            .spclient()
            .request(&http::Method::GET, &list_endpoint, None, None)
            .await?;
        let revision = SelectedListContent::parse_from_bytes(&bytes)
            .map_err(|e| EditError::Server(format!("kök liste çözülemedi: {e}")))?
            .revision()
            .to_vec();
        let body = build(&revision);
        match session
            .spclient()
            .request_with_protobuf(&http::Method::POST, &endpoint, None, &body)
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => match EditError::from_spotify(&e) {
                e @ EditError::Conflict(_) => last = Some(e),
                e => return Err(e),
            },
        }
    }
    Err(last.unwrap_or_else(|| EditError::Conflict("kök liste sürüm çakışması".into())))
}

/// Kök listeyi okuyup planı uygular; `build` `None` derse yazılacak bir şey
/// yok (işlem zaten olmuş). Sürüm çakışırsa bir kez daha dener (bkz.
/// `post_playlist_changes`).
async fn post_rootlist_plan(
    session: &Session,
    build: impl Fn(&RootState) -> Result<Option<ListChanges>, EditError>,
) -> Result<(), EditError> {
    let user = session.username();
    let endpoint = format!("/playlist/v2/user/{user}/rootlist/changes");
    let mut last = None;
    for _ in 0..2 {
        let state = rootlist_state(session).await?;
        let Some(body) = build(&state)? else {
            return Ok(());
        };
        match session
            .spclient()
            .request_with_protobuf(&http::Method::POST, &endpoint, None, &body)
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => match EditError::from_spotify(&e) {
                e @ EditError::Conflict(_) => last = Some(e),
                e => return Err(e),
            },
        }
    }
    Err(last.unwrap_or_else(|| EditError::Conflict("kök liste sürüm çakışması".into())))
}

/// Yeni (boş) listeye adı ve şarkıları tek değişiklikte yazar; zaten yazılmış
/// olan parça (ad aynı / şarkılar dolu) atlanır.
async fn fill_new_playlist(
    session: &Session,
    playlist: &str,
    name: &str,
    uris: &[String],
) -> Result<(), EditError> {
    post_playlist_changes(session, playlist, |st| {
        changes_fill(&st.revision, (st.name != name).then_some(name), {
            if st.uris.is_empty() { uris } else { &[] }
        })
    })
    .await
    .map(|_| ())
}

/// Ad (isteğe bağlı) ve şarkıları (sona) tek delta'da yazar; yazılacak bir
/// şey yoksa `None`.
pub fn changes_fill(base: &[u8], name: Option<&str>, uris: &[String]) -> Option<ListChanges> {
    let mut ops = Vec::new();
    if let Some(name) = name {
        ops.extend(changes_rename(base, name).deltas.remove(0).ops);
    }
    if !uris.is_empty() {
        ops.extend(
            changes_add(base, uris, AddPosition::Last)
                .deltas
                .remove(0)
                .ops,
        );
    }
    (!ops.is_empty()).then(|| changes(base, ops))
}

async fn create_playlist(
    session: &Session,
    name: &str,
    uris: &[String],
) -> Result<String, EditError> {
    let body = create_request(name, uris);
    let bytes = session
        .spclient()
        .request_with_protobuf(&http::Method::POST, "/playlist/v2/playlist", None, &body)
        .await?;
    let reply = librespot::protocol::playlist4_external::CreateListReply::parse_from_bytes(&bytes)
        .map_err(|e| EditError::Server(format!("oluşturma yanıtı çözülemedi: {e}")))?;
    let uri = reply.uri().to_string();
    if uri.is_empty() {
        return Err(EditError::Server("Spotify liste adresi vermedi".into()));
    }
    Ok(uri)
}

// ── Kalıcı yeniden deneme kuyruğu ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditState {
    /// Sırada (ya da bir sonraki deneme zamanını bekliyor).
    Pending,
    /// Kalıcı olarak yazılamadı; kullanıcı görüp "tekrar dene" ya da "bırak"
    /// diyene kadar durur. Hiçbir zaman kendiliğinden silinmez.
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedEdit {
    pub id: i64,
    pub op: EditOp,
    pub state: EditState,
    pub attempts: u32,
    pub next_attempt_ms: i64,
    pub last_error: Option<String>,
    /// Son hatanın türü (başarısız / yeniden denenen işlemlerde).
    pub error_kind: Option<FailKind>,
    pub maybe_applied: bool,
    /// Şu an Spotify'a gönderiliyor (sonucu henüz bilinmiyor): Spotify'dan o
    /// anda okunan veri bu işlemin etkisini içerebilir de, içermeyebilir de.
    pub in_flight: bool,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enqueued {
    Queued(i64),
    /// Sıradaki zıt işlemle birbirini götürdü (beğen + vazgeç): hiçbir şey yazılmayacak.
    Cancelled,
    /// Aynısı zaten sırada.
    Duplicate(i64),
}

#[derive(Debug, Default, PartialEq)]
pub struct ProcessReport {
    pub done: Vec<(i64, EditOutcome)>,
    /// `done` ile aynı sırada: yazılan işlemin kendisi (oluşturma sonrası
    /// geçici adresi gerçeğe çevrilmiş hâliyle). Motor, yerel önbelleği
    /// bununla Spotify'daki hâle yetiştirir.
    pub done_ops: Vec<EditOp>,
    /// Bu turda yeniden denemeye bırakılanlar.
    pub retry_later: Vec<i64>,
    /// Kalıcı olarak başarısız olanlar (kullanıcıya gösterilecek).
    pub failed: Vec<i64>,
    /// Bağlantı yoktu: tur erken bitti.
    pub offline: bool,
    /// En erken bir sonraki deneme zamanı (zamanlayıcı için).
    pub next_due_ms: Option<i64>,
}

/// Kuyruk şeması (`migrate.rs`). Kuyruk KULLANICI verisidir: hiçbir adım satır
/// silmez, yalnız ekler. Daha yeni bir uygulamanın yazdığı dosya açılmaz ve
/// dokunulmaz (o oturumda kuyruk yok; veri kaybolmaz).
///   1  temel şema (göçten önce sürümlenmemiş dosyalar da burada devralınır)
///   2  `error_kind` sütunu (eski kayıtlarda boş: tür bilinmiyor)
const MIGRATIONS: [Step; 2] = [
    Step {
        to: 1,
        name: "baseline",
        up: baseline,
    },
    Step {
        to: 2,
        name: "error_kind column",
        up: add_error_kind,
    },
];

fn baseline(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS edits (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             op              TEXT NOT NULL,
             target          TEXT NOT NULL,
             state           TEXT NOT NULL,
             attempts        INTEGER NOT NULL DEFAULT 0,
             next_attempt_ms INTEGER NOT NULL DEFAULT 0,
             last_error      TEXT,
             maybe_applied   INTEGER NOT NULL DEFAULT 0,
             in_flight       INTEGER NOT NULL DEFAULT 0,
             created_ms      INTEGER NOT NULL
         );",
    )
}

fn add_error_kind(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    // Sürümlenmeden önce sütunu ekleyen eski sürümlerin dosyalarında zaten var.
    let has_kind = tx
        .prepare("SELECT 1 FROM pragma_table_info('edits') WHERE name = 'error_kind'")?
        .exists([])?;
    if !has_kind {
        tx.execute("ALTER TABLE edits ADD COLUMN error_kind TEXT", [])?;
    }
    Ok(())
}

pub struct EditQueue {
    conn: Mutex<Connection>,
}

impl EditQueue {
    /// Kuyruk, KULLANICI verisidir, önbellek değil: önbellek klasöründe
    /// durmaz (silinirse kayıp olurdu). Varsayılan yer ayar dosyasının yanı
    /// (`%APPDATA%\Openify\edits.db`).
    pub fn open_default() -> Result<Self> {
        let dir = dirs::data_dir()
            .context("Veri klasörü bulunamadı")?
            .join("Openify");
        std::fs::create_dir_all(&dir)?;
        Self::open(&dir.join("edits.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path).context("Yazma kuyruğu açılamadı")?)
    }

    /// Bellekte kuyruk: birim testleri ve arayüz sınamaları (sınama koşusunda
    /// hiçbir işlem diske ya da Spotify'a gitmez, bkz. `wire::test_edits`).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // Kuyrukta veri kaybı olmasın: tam eşzamanlı yazma.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        migrate::migrate(&mut conn, &MIGRATIONS).context("Yazma kuyruğu şeması yükseltilemedi")?;
        // Uygulama gönderim sırasında kapandıysa o işlemin sunucuya ulaşıp
        // ulaşmadığı bilinmez: belirsiz diye işaretle (uygulamadan önce denetlenir).
        conn.execute(
            "UPDATE edits SET in_flight = 0, maybe_applied = 1 WHERE in_flight = 1",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// İşlemi kalıcı olarak sıraya yazar. Dönüşte işlem diske yazılmıştır.
    pub fn enqueue(&self, op: &EditOp) -> Result<Enqueued, EditError> {
        self.enqueue_at(op, now_ms())
    }

    fn enqueue_at(&self, op: &EditOp, now: i64) -> Result<Enqueued, EditError> {
        op.validate()?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(db_err)?;
        let json = serde_json::to_string(op).map_err(|e| EditError::Invalid(e.to_string()))?;
        let target = op.target();

        // Aynı hedefte, henüz hiç denenmemiş, sırada bekleyen zıt/aynı işlem var mı?
        let existing: Option<(i64, String)> = {
            let opposite = match op {
                EditOp::Save { uri } => Some(EditOp::Unsave { uri: uri.clone() }),
                EditOp::Unsave { uri } => Some(EditOp::Save { uri: uri.clone() }),
                EditOp::Follow { uri } => Some(EditOp::Unfollow { uri: uri.clone() }),
                EditOp::Unfollow { uri } => Some(EditOp::Follow { uri: uri.clone() }),
                _ => None,
            };
            let mut found = None;
            let mut stmt = tx
                .prepare(
                    "SELECT id, op FROM edits
                     WHERE target = ?1 AND state = 'pending' AND in_flight = 0
                       AND maybe_applied = 0 AND attempts = 0
                     ORDER BY id",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([&target], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(db_err)?;
            for row in rows {
                let (id, stored) = row.map_err(db_err)?;
                let Ok(stored_op) = serde_json::from_str::<EditOp>(&stored) else {
                    continue;
                };
                if &stored_op == op {
                    found = Some((id, "same".to_string()));
                } else if opposite.as_ref() == Some(&stored_op) {
                    found = Some((id, "opposite".to_string()));
                }
            }
            found
        };
        match existing {
            Some((id, kind)) if kind == "same" => {
                tx.commit().map_err(db_err)?;
                return Ok(Enqueued::Duplicate(id));
            }
            Some((id, _)) => {
                tx.execute("DELETE FROM edits WHERE id = ?1", [id])
                    .map_err(db_err)?;
                tx.commit().map_err(db_err)?;
                return Ok(Enqueued::Cancelled);
            }
            None => {}
        }

        tx.execute(
            "INSERT INTO edits (op, target, state, created_ms) VALUES (?1, ?2, 'pending', ?3)",
            params![json, target, now],
        )
        .map_err(db_err)?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(db_err)?;
        Ok(Enqueued::Queued(id))
    }

    fn row(r: &rusqlite::Row) -> rusqlite::Result<QueuedEdit> {
        let json: String = r.get(1)?;
        let state: String = r.get(2)?;
        let op = serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(QueuedEdit {
            id: r.get(0)?,
            op,
            state: if state == "failed" {
                EditState::Failed
            } else {
                EditState::Pending
            },
            attempts: r.get::<_, i64>(3)? as u32,
            next_attempt_ms: r.get(4)?,
            last_error: r.get(5)?,
            maybe_applied: r.get::<_, i64>(6)? != 0,
            created_ms: r.get(7)?,
            in_flight: r.get::<_, i64>(8)? != 0,
            error_kind: r
                .get::<_, Option<String>>(9)?
                .map(|t| FailKind::from_tag(&t)),
        })
    }

    const COLUMNS: &'static str = "id, op, state, attempts, next_attempt_ms, last_error, \
                                   maybe_applied, created_ms, in_flight, error_kind";

    fn query(&self, filter: &str) -> Result<Vec<QueuedEdit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM edits WHERE {filter} ORDER BY id",
            Self::COLUMNS
        ))?;
        Ok(stmt
            .query_map([], Self::row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<QueuedEdit>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {} FROM edits WHERE id = ?1", Self::COLUMNS),
                [id],
                Self::row,
            )
            .optional()?)
    }

    /// Sırada bekleyenler (arayüzde "eşitleniyor" göstergesi için).
    pub fn pending(&self) -> Result<Vec<QueuedEdit>> {
        self.query("state = 'pending'")
    }

    /// Yazılamayanlar (arayüz açıkça gösterir).
    pub fn failed(&self) -> Result<Vec<QueuedEdit>> {
        self.query("state = 'failed'")
    }

    /// Kullanıcı başarısız işlemi bıraktı: işlemi bırakır ve bırakılanları (işlem kayıtları) döndürür.
    /// Oluşturma bırakılırsa, o geçici listeye bağlı yazılamayanlar da
    /// bırakılır: liste hiç oluşmayacak, onlara yapacak iş kalmadı. Bekleyen
    /// ya da olmayan işleme dokunmaz (boş döner).
    pub fn dismiss_cascade(&self, id: i64) -> Result<Vec<QueuedEdit>> {
        let Some(edit) = self.get(id)?.filter(|e| e.state == EditState::Failed) else {
            return Ok(Vec::new());
        };
        let mut removed = vec![edit.clone()];
        if let EditOp::CreatePlaylist { temp_id, .. } = &edit.op {
            for dep in self.query(&format!(
                "state = 'failed' AND target = '{}'",
                temp_id.replace('\'', "''")
            ))? {
                if dep.id != id {
                    removed.push(dep);
                }
            }
        }
        for e in &removed {
            self.conn().execute(
                "DELETE FROM edits WHERE id = ?1 AND state = 'failed'",
                [e.id],
            )?;
        }
        Ok(removed)
    }

    /// Kullanıcı başarısız işlemi yeniden denemek istedi.
    pub fn retry(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE edits SET state = 'pending', attempts = 0, next_attempt_ms = 0,
                              last_error = NULL, error_kind = NULL, created_ms = ?2
             WHERE id = ?1 AND state = 'failed'",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    /// Bu hedefin geçici adresi gerçek adresle değişti: ona bağlı bekleyen
    /// işlemleri yeniden yaz.
    fn rewrite_dependents(&self, temp: &str, real: &str) -> Result<()> {
        for edit in self.query(&format!("target = '{}'", temp.replace('\'', "''")))? {
            let mut op = edit.op.clone();
            op.rewrite_target(temp, real);
            if op != edit.op {
                self.conn().execute(
                    "UPDATE edits SET op = ?2, target = ?3 WHERE id = ?1",
                    params![edit.id, serde_json::to_string(&op)?, op.target()],
                )?;
            }
        }
        Ok(())
    }

    fn set_in_flight(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("UPDATE edits SET in_flight = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    fn finish_ok(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM edits WHERE id = ?1", [id])?;
        Ok(())
    }

    fn record_failure(
        &self,
        edit: &QueuedEdit,
        error: &EditError,
        retry_at: Option<i64>,
    ) -> Result<()> {
        // Yarım kalmış oluşturmanın adresi işleme yazılır.
        let mut op = edit.op.clone();
        if let (Some(uri), EditOp::CreatePlaylist { created, .. }) =
            (error.partial_create(), &mut op)
        {
            *created = Some(uri.to_string());
        }
        self.conn().execute(
            "UPDATE edits SET op = ?2, state = ?3, attempts = ?4, next_attempt_ms = ?5,
                              last_error = ?6, maybe_applied = ?7, in_flight = 0,
                              error_kind = ?8
             WHERE id = ?1",
            params![
                edit.id,
                serde_json::to_string(&op)?,
                if retry_at.is_some() {
                    "pending"
                } else {
                    "failed"
                },
                edit.attempts + 1,
                retry_at.unwrap_or(0),
                error.to_string(),
                i64::from(error.may_have_applied() || edit.maybe_applied && retry_at.is_some()),
                error.kind().tag(),
            ],
        )?;
        Ok(())
    }

    /// Bir işlem kalıcı başarısız olunca ona bağlı bekleyenleri de (kendi
    /// gerekçeleriyle) başarısız yapar: hiçbiri sessizce kalmaz.
    fn fail_dependents(&self, temp: &str, why: &str) -> Result<Vec<i64>> {
        let mut failed = Vec::new();
        for dep in self.query(&format!("target = '{}'", temp.replace('\'', "''")))? {
            if matches!(dep.op, EditOp::CreatePlaylist { .. }) {
                continue;
            }
            self.conn().execute(
                "UPDATE edits SET state = 'failed', last_error = ?2, error_kind = ?3 WHERE id = ?1",
                params![
                    dep.id,
                    format!("Bağlı olduğu liste oluşturulamadı: {why}"),
                    FailKind::ParentFailed.tag()
                ],
            )?;
            failed.push(dep.id);
        }
        Ok(failed)
    }

    /// Zamanı gelen işlemleri sırayla Spotify'a yazar.
    ///
    /// - Aynı hedefe yazılanlar kayıt sırasıyla; biri yeniden denemeye kalırsa
    ///   o hedefin sonrakileri de bekler (sıra bozulmaz). Farklı hedefler
    ///   birbirini beklemez.
    /// - Bağlantı yoksa tur hemen biter (boşuna istek atılmaz).
    /// - Kilit, ağ çağrısı boyunca tutulmaz.
    pub async fn process_due<B: EditBackend>(
        &self,
        backend: &B,
        now: i64,
    ) -> Result<ProcessReport> {
        let mut report = ProcessReport::default();
        let mut blocked: HashSet<String> = HashSet::new();
        let ids: Vec<i64> = self.pending()?.into_iter().map(|e| e.id).collect();

        for id in ids {
            // Önceki işlemler satırı değiştirmiş / silmiş olabilir: güncelini oku.
            let Some(edit) = self.get(id)? else { continue };
            if edit.state != EditState::Pending {
                continue;
            }
            let target = edit.op.target();
            if blocked.contains(&target) {
                continue;
            }
            if edit.next_attempt_ms > now {
                blocked.insert(target);
                report.next_due_ms = Some(
                    report
                        .next_due_ms
                        .map_or(edit.next_attempt_ms, |t| t.min(edit.next_attempt_ms)),
                );
                continue;
            }
            if now - edit.created_ms > MAX_AGE_MS {
                let error = EditError::Expired(format!(
                    "{} gündür yazılamadı",
                    (now - edit.created_ms) / 86_400_000
                ));
                self.record_failure(&edit, &error, None)?;
                report.failed.push(id);
                continue;
            }

            self.set_in_flight(id)?;
            let ctx = ApplyCtx {
                id,
                maybe_applied: edit.maybe_applied,
            };
            match backend.apply(&edit.op, ctx).await {
                Ok(outcome) => {
                    self.finish_ok(id)?;
                    if let (EditOp::CreatePlaylist { temp_id, .. }, EditOutcome::Created { uri }) =
                        (&edit.op, &outcome)
                    {
                        self.rewrite_dependents(temp_id, uri)?;
                    }
                    report.done.push((id, outcome));
                    report.done_ops.push(edit.op.clone());
                }
                Err(error) => {
                    let attempts = edit.attempts + 1;
                    if error.is_retryable(attempts) {
                        let at = now + backoff_ms(attempts, &error);
                        self.record_failure(&edit, &error, Some(at))?;
                        report.retry_later.push(id);
                        report.next_due_ms =
                            Some(report.next_due_ms.map_or(at, |t: i64| t.min(at)));
                        blocked.insert(target);
                        if error.is_offline() {
                            report.offline = true;
                            break;
                        }
                    } else {
                        self.record_failure(&edit, &error, None)?;
                        report.failed.push(id);
                        if let EditOp::CreatePlaylist { temp_id, .. } = &edit.op {
                            report
                                .failed
                                .extend(self.fail_dependents(temp_id, &error.to_string())?);
                        }
                    }
                }
            }
        }
        Ok(report)
    }
}

fn db_err(e: rusqlite::Error) -> EditError {
    EditError::Invalid(format!("Yazma kuyruğu veritabanı hatası: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn track(n: u32) -> String {
        format!("spotify:track:{n:022}")
    }

    fn playlist(n: u32) -> String {
        format!("spotify:playlist:{n:022}")
    }

    // ── İstek gövdeleri ─────────────────────────────────────────────

    #[test]
    fn collection_write_roundtrip() {
        let body = collection_write_body(
            "kullanici",
            &[track(1), track(2)],
            true,
            1_700_000_000,
            "u1",
        )
        .unwrap();
        let mut is = protobuf::CodedInputStream::from_bytes(&body);
        let mut username = String::new();
        let mut set = String::new();
        let mut items = Vec::new();
        let mut update_id = String::new();
        while let Some(tag) = is.read_raw_tag_or_eof().unwrap() {
            match tag >> 3 {
                1 => username = is.read_string().unwrap(),
                2 => set = is.read_string().unwrap(),
                3 => {
                    let raw = is.read_bytes().unwrap();
                    let mut it = protobuf::CodedInputStream::from_bytes(&raw);
                    let (mut uri, mut at, mut removed) = (String::new(), 0, false);
                    while let Some(t) = it.read_raw_tag_or_eof().unwrap() {
                        match t >> 3 {
                            1 => uri = it.read_string().unwrap(),
                            2 => at = it.read_int32().unwrap(),
                            3 => removed = it.read_bool().unwrap(),
                            _ => panic!("beklenmeyen alan"),
                        }
                    }
                    items.push((uri, at, removed));
                }
                4 => update_id = is.read_string().unwrap(),
                _ => panic!("beklenmeyen alan"),
            }
        }
        assert_eq!(username, "kullanici");
        assert_eq!(set, "collection");
        assert_eq!(update_id, "u1");
        assert_eq!(
            items,
            vec![
                (track(1), 1_700_000_000, true),
                (track(2), 1_700_000_000, true)
            ]
        );
        // Ekleme gövdesinde is_removed hiç yazılmaz.
        let add = collection_write_body("k", &[track(1)], false, 5, "x").unwrap();
        assert!(!add.windows(2).any(|w| w == [0x18, 0x01]));
    }

    fn roundtrip(c: &ListChanges) -> ListChanges {
        ListChanges::parse_from_bytes(&c.write_to_bytes().unwrap()).unwrap()
    }

    #[test]
    fn add_changes_encode_position_and_items() {
        let c = roundtrip(&changes_add(
            b"rev1",
            &[track(1), track(2)],
            AddPosition::Last,
        ));
        assert_eq!(c.base_revision(), b"rev1");
        assert!(c.want_resulting_revisions());
        let op = &c.deltas[0].ops[0];
        assert_eq!(op.kind(), Kind::ADD);
        assert!(op.add.add_last());
        assert!(!op.add.add_first());
        assert_eq!(
            op.add.items.iter().map(|i| i.uri()).collect::<Vec<_>>(),
            [track(1), track(2)]
        );
        let first = roundtrip(&changes_add(b"", &[track(1)], AddPosition::First));
        assert!(first.deltas[0].ops[0].add.add_first());
        // Sürüm yoksa alan hiç yazılmaz.
        assert!(!first.has_base_revision());
    }

    #[test]
    fn remove_rename_move_changes() {
        let c = roundtrip(&changes_remove(b"r", &[track(7)]));
        let op = &c.deltas[0].ops[0];
        assert_eq!(op.kind(), Kind::REM);
        assert!(op.rem.items_as_key());
        assert_eq!(op.rem.items[0].uri(), track(7));

        let c = roundtrip(&changes_rename(b"r", "Yeni Ad"));
        let op = &c.deltas[0].ops[0];
        assert_eq!(op.kind(), Kind::UPDATE_LIST_ATTRIBUTES);
        assert_eq!(
            op.update_list_attributes.new_attributes.values.name(),
            "Yeni Ad"
        );

        let c = roundtrip(&changes_move(b"r", 3, 2, 8));
        let op = &c.deltas[0].ops[0];
        assert_eq!(op.kind(), Kind::MOV);
        assert_eq!(
            (op.mov.from_index(), op.mov.length(), op.mov.to_index()),
            (3, 2, 10)
        );
        // Geri taşımada teldeki değer istenen son yerle aynı.
        let c = roundtrip(&changes_move(b"r", 8, 2, 3));
        assert_eq!(c.deltas[0].ops[0].mov.to_index(), 3);
    }

    #[test]
    fn create_request_and_rootlist_ops() {
        let r = ListUpdateRequest::parse_from_bytes(
            &create_request("Yolculuk", &[track(1)])
                .write_to_bytes()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(r.attributes.name(), "Yolculuk");
        assert_eq!(r.items[0].uri(), track(1));

        let add = roundtrip(&rootlist_add(b"r", &playlist(1)));
        assert!(add.deltas[0].ops[0].add.add_first());
        assert_eq!(add.deltas[0].ops[0].add.items[0].uri(), playlist(1));
        let rem = roundtrip(&rootlist_remove(b"r", &playlist(1)));
        assert_eq!(rem.deltas[0].ops[0].kind(), Kind::REM);
    }

    #[test]
    fn playlist_ids_are_validated() {
        assert_eq!(playlist_id(&playlist(5)).unwrap(), format!("{:022}", 5));
        assert!(playlist_id("spotify:album:abc").is_err());
        assert!(playlist_id("spotify:playlist:kisa").is_err());
        assert!(playlist_id(&new_temp_id()).is_err());
    }

    // ── Liste ayarları ve klasörler ─────────────────────────────────

    #[test]
    fn description_and_collaborative_changes() {
        let c = roundtrip(&changes_description(b"r", "Gece için"));
        let op = &c.deltas[0].ops[0];
        assert_eq!(op.kind(), Kind::UPDATE_LIST_ATTRIBUTES);
        let attrs = &op.update_list_attributes.new_attributes.values;
        assert_eq!(attrs.description(), "Gece için");
        assert!(op.update_list_attributes.new_attributes.no_value.is_empty());
        // Boş açıklama alanı temizler: partial state "bu alan yok" der.
        let c = roundtrip(&changes_description(b"r", ""));
        let na = &c.deltas[0].ops[0].update_list_attributes.new_attributes;
        assert_eq!(na.values.description(), "");
        let kinds: Vec<ListAttributeKind> = na
            .no_value
            .iter()
            .map(|k| k.enum_value().expect("bilinen tür"))
            .collect();
        assert_eq!(kinds, [ListAttributeKind::LIST_DESCRIPTION]);
        // Ortak liste: iki değer de açıkça yazılır (kapalı da bir değerdir).
        for on in [true, false] {
            let c = roundtrip(&changes_collaborative(b"r", on));
            let attrs = &c.deltas[0].ops[0].update_list_attributes.new_attributes;
            assert_eq!(attrs.values.collaborative(), on);
            assert!(attrs.values.has_collaborative());
            assert!(attrs.no_value.is_empty());
        }
        // Sınır: 300 karakter kabul, 301 reddedilir.
        let op = |n: usize| EditOp::SetPlaylistDescription {
            playlist: playlist(1),
            description: "a".repeat(n),
            previous: None,
        };
        assert!(op(DESCRIPTION_MAX).validate().is_ok());
        assert!(op(DESCRIPTION_MAX + 1).validate().is_err());
    }

    #[test]
    fn folder_markers_are_percent_encoded_and_read_back() {
        use super::super::library::{Tok, parse_marker};
        let key = folder_start_marker("abc123", "Gece / Yaz: 90'lar %");
        assert!(key.starts_with("spotify:start-group:abc123:"));
        // `:` adın içinde de geçse kimlik ilk parçadır.
        let Some(Tok::Start {
            id,
            name,
            key: full,
        }) = parse_marker(&key)
        else {
            panic!("klasör işareti okunamadı: {key}");
        };
        assert_eq!(id, "abc123");
        assert_eq!(name, "Gece / Yaz: 90'lar %");
        assert_eq!(full, key);
        assert_eq!(folder_marker_id(&key), Some("abc123"));
        assert_eq!(folder_marker_id("spotify:end-group:abc"), None);
        assert_eq!(
            renamed_folder_key(&key, "Kış").as_deref(),
            Some(folder_start_marker("abc123", "Kış").as_str())
        );
        assert_eq!(
            parse_marker(&folder_end_marker("abc123")),
            Some(Tok::End {
                id: "abc123".into()
            })
        );
        // Boş ad ve kimlikte özel karakter yok.
        assert_eq!(encode_folder_name("Yaz"), "Yaz");
        assert_eq!(encode_folder_name("a b"), "a%20b");
        // Yeni kimlikler benzersiz ve onaltılık.
        let (a, b) = (new_folder_id(), new_folder_id());
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    }

    /// Klasör planları: aç → içine taşı → adlandır → sil → geri getir.
    /// (Saf: ağ yok; sonuç listesi `apply_move` ile aynı anlamda.)
    #[test]
    fn folder_plans_create_move_rename_delete_restore() {
        let (a, b, c) = (playlist(1), playlist(2), playlist(3));
        let base = vec![a.clone(), b.clone(), c.clone()];
        let key = folder_start_marker("f1", "Yaz");
        let end = folder_end_marker("f1");

        // Aç: sona iki işaret.
        let (opened, ops) = folder_create_ops(&base, &key).unwrap();
        assert_eq!(
            opened,
            [a.clone(), b.clone(), c.clone(), key.clone(), end.clone()]
        );
        assert_eq!(ops[0].kind(), Kind::ADD);
        assert!(ops[0].add.add_last());
        // İkinci kez açılmaz (idempotent).
        assert!(folder_create_ops(&opened, &key).is_none());

        // İçine taşı: bitiş işaretinin hemen önü.
        let (inside, _) = playlist_move_ops(&opened, &c, Some(key.as_str()))
            .unwrap()
            .unwrap();
        assert_eq!(
            inside,
            [a.clone(), b.clone(), key.clone(), c.clone(), end.clone()]
        );
        assert_eq!(parent_folder_at(&inside, 3).as_deref(), Some(key.as_str()));
        assert!(
            playlist_move_ops(&inside, &c, Some(key.as_str()))
                .unwrap()
                .is_none()
        );

        // Klasörden çıkar: kök listenin sonu.
        let (root, _) = playlist_move_ops(&inside, &c, None).unwrap().unwrap();
        assert_eq!(
            root,
            [a.clone(), b.clone(), key.clone(), end.clone(), c.clone()]
        );
        assert_eq!(parent_folder_at(&root, 4), None);

        // Yeniden adlandır: işaret aynı yerde.
        let (renamed, _) = folder_rename_ops(&root, &key, "Kış").unwrap().unwrap();
        let key2 = folder_start_marker("f1", "Kış");
        assert_eq!(
            renamed,
            [a.clone(), b.clone(), key2.clone(), end, c.clone()]
        );
        assert!(folder_rename_ops(&renamed, &key, "Kış").unwrap().is_none());
        assert!(matches!(
            folder_rename_ops(&renamed, "spotify:start-group:yok:Yok", "X"),
            Err(EditError::NotFound(_))
        ));

        // Sil: işaretler kalkar, içindekiler yerinde.
        let (gone, _) = folder_delete_ops(&renamed, &key2).unwrap();
        assert_eq!(gone, [a.clone(), b.clone(), c.clone()]);
        assert!(folder_delete_ops(&gone, &key2).is_none());

        // Geri getir: eski yerine, içeriği sırayla.
        let (back, _) = folder_restore_ops(&gone, &key2, Some(c.as_str()), &[b.clone()]).unwrap();
        assert_eq!(
            back,
            [a, key2.clone(), b, folder_end_marker("f1"), c.clone()]
        );
        assert_eq!(parent_folder_at(&back, 2).as_deref(), Some(key2.as_str()));
        assert!(folder_restore_ops(&back, &key2, Some(c.as_str()), &[]).is_none());
    }

    /// İç içe klasör: dış klasör silinince içteki klasör ve listeleri
    /// yerinde kalır; taşıma yalnız doğrudan sınırları değiştirir.
    #[test]
    fn nested_folder_plans_keep_the_inner_folder() {
        let (a, b, c) = (playlist(1), playlist(2), playlist(3));
        let outer = folder_start_marker("o", "Dış");
        let inner = folder_start_marker("i", "İç");
        let uris = vec![
            a.clone(),
            outer.clone(),
            b.clone(),
            inner.clone(),
            c.clone(),
            folder_end_marker("i"),
            folder_end_marker("o"),
        ];
        assert_eq!(parent_folder_at(&uris, 2).as_deref(), Some(outer.as_str()));
        assert_eq!(parent_folder_at(&uris, 4).as_deref(), Some(inner.as_str()));
        // İçteki listeyi dış klasörün içine (iç klasörün dışına) çıkar.
        let (moved, _) = playlist_move_ops(&uris, &c, Some(outer.as_str()))
            .unwrap()
            .unwrap();
        assert_eq!(
            moved,
            [
                a.clone(),
                outer.clone(),
                b.clone(),
                inner.clone(),
                folder_end_marker("i"),
                c.clone(),
                folder_end_marker("o"),
            ]
        );
        assert_eq!(parent_folder_at(&moved, 5).as_deref(), Some(outer.as_str()));
        // Dış klasör silinince içteki klasör (ve içindekiler) durur.
        let (gone, _) = folder_delete_ops(&uris, &outer).unwrap();
        assert_eq!(gone, [a, b, inner, c, folder_end_marker("i")]);
    }

    #[test]
    fn folder_ops_target_rootlist_and_go_through_json() {
        let key = folder_start_marker("f1", "Yaz");
        let ops = vec![
            EditOp::CreateFolder {
                key: key.clone(),
                name: "Yaz".into(),
            },
            EditOp::RenameFolder {
                folder: key.clone(),
                name: "Kış".into(),
                previous_name: Some("Yaz".into()),
            },
            EditOp::DeleteFolder {
                folder: key.clone(),
                name: "Kış".into(),
                before: Some(playlist(1)),
                contents: vec![playlist(2)],
            },
            EditOp::RestoreFolder {
                folder: key.clone(),
                name: "Kış".into(),
                before: Some(playlist(1)),
                contents: vec![playlist(2)],
            },
            EditOp::MovePlaylistToFolder {
                playlist: playlist(1),
                folder: Some(key.clone()),
                previous: None,
            },
        ];
        for op in &ops {
            assert_eq!(op.target(), ROOTLIST_TARGET);
            let json = serde_json::to_string(op).unwrap();
            assert_eq!(&serde_json::from_str::<EditOp>(&json).unwrap(), op);
        }
        // Tersler: aç → sil, sil → geri getir, ad → eski ad, taşı → eski yer.
        assert!(matches!(
            ops[0].inverse().unwrap(),
            EditOp::DeleteFolder { ref folder, .. } if *folder == key
        ));
        assert!(matches!(
            ops[2].inverse().unwrap(),
            EditOp::RestoreFolder { ref folder, .. } if *folder == key
        ));
        let EditOp::RenameFolder { folder, name, .. } = ops[1].inverse().unwrap() else {
            panic!()
        };
        let renamed = folder_start_marker("f1", "Kış");
        assert_eq!((folder.as_str(), name.as_str()), (renamed.as_str(), "Yaz"));
        let EditOp::MovePlaylistToFolder {
            folder, previous, ..
        } = ops[4].inverse().unwrap()
        else {
            panic!()
        };
        assert_eq!((folder, previous), (None, Some(key)));
    }

    // ── Yerel sıralama ──────────────────────────────────────────────

    #[test]
    fn apply_move_matches_final_position_semantics() {
        let base: Vec<u32> = (0..6).collect();
        let mut v = base.clone();
        assert!(apply_move(&mut v, 1, 2, 3));
        assert_eq!(v, [0, 3, 4, 1, 2, 5]);
        // Geri alma (inverse) eski sıraya döndürür.
        let undo = EditOp::MovePlaylistItems {
            playlist: playlist(1),
            from_index: 1,
            length: 2,
            to_index: 3,
            uris: vec![],
        }
        .inverse()
        .unwrap();
        let EditOp::MovePlaylistItems {
            from_index,
            length,
            to_index,
            ..
        } = undo
        else {
            panic!()
        };
        assert!(apply_move(
            &mut v,
            from_index as usize,
            length as usize,
            to_index as usize
        ));
        assert_eq!(v, base);
        // Sınır dışı: dokunulmaz.
        let mut w = base.clone();
        assert!(!apply_move(&mut w, 4, 3, 0));
        assert!(!apply_move(&mut w, 0, 2, 5));
        assert!(!apply_move(&mut w, 0, 0, 1));
        assert_eq!(w, base);
        // Başa ve sona taşıma.
        let mut x = base.clone();
        assert!(apply_move(&mut x, 5, 1, 0));
        assert_eq!(x, [5, 0, 1, 2, 3, 4]);
        assert!(apply_move(&mut x, 0, 1, 5));
        assert_eq!(x, base);
    }

    /// Canlı doğrulanan anlam: sunucu bloğu çıkarmadan ÖNCEKİ listedeki
    /// `wire` indisli öğenin önüne ekler. Bu modelle, `wire_to_index`'in
    /// ürettiği değer her (from, len, to) için `apply_move` ile aynı sırayı vermeli.
    #[test]
    fn mov_wire_semantics_match_live_result() {
        fn server_mov(order: &[u32], from: usize, len: usize, wire: usize) -> Vec<u32> {
            let anchor = order.get(wire).copied();
            let block: Vec<u32> = order[from..from + len].to_vec();
            let mut rest: Vec<u32> = order.to_vec();
            rest.drain(from..from + len);
            let at = match anchor {
                Some(a) if !block.contains(&a) => rest.iter().position(|x| *x == a).unwrap(),
                Some(_) => from, // hedef bloğun içinde: yerinde kalır
                None => rest.len(),
            };
            for (i, x) in block.into_iter().enumerate() {
                rest.insert(at + i, x);
            }
            rest
        }
        // Canlıda görülen: [A,D,B,C] listesinde tel değeri 2 gönderilince
        // [D,A,B,C] çıktı. Arayüz "son yer 2" istiyorsa ([D,B,A,C]) tel değeri 3.
        assert_eq!(wire_to_index(0, 1, 2), 3);
        assert_eq!(server_mov(&[0, 1, 2, 3], 0, 1, 2), [1, 0, 2, 3]);
        assert_eq!(server_mov(&[0, 1, 2, 3], 0, 1, 3), [1, 2, 0, 3]);
        let n = 7usize;
        let base: Vec<u32> = (0..n as u32).collect();
        for from in 0..n {
            for len in 1..=(n - from) {
                for to in 0..=(n - len) {
                    if to == from {
                        continue;
                    }
                    let mut want = base.clone();
                    assert!(apply_move(&mut want, from, len, to));
                    let wire = wire_to_index(from as u32, len as u32, to as u32) as usize;
                    assert_eq!(
                        server_mov(&base, from, len, wire),
                        want,
                        "from {from} len {len} to {to} (tel {wire})"
                    );
                }
            }
        }
    }

    #[test]
    fn single_row_removal_picks_exactly_one_copy() {
        // [a b a c a]: üç kopya.
        let state = vec![track(1), track(2), track(1), track(3), track(1)];
        // Sıra verilmemiş: ilk kopya.
        assert_eq!(
            resolve_rows(&state, &[track(1)], &[], false),
            [(0, track(1))]
        );
        // Sıra verilmiş ve doğru: o satır.
        assert_eq!(
            resolve_rows(&state, &[track(1)], &[4], false),
            [(4, track(1))]
        );
        // Sıra verilmiş ama orada başka şarkı var: ilk kopyaya düşer (liste kaymış).
        assert_eq!(
            resolve_rows(&state, &[track(1)], &[3], false),
            [(0, track(1))]
        );
        // Belirsiz yeniden denemede (strict) kaymış sıra başka kopyayı silmez.
        assert!(resolve_rows(&state, &[track(1)], &[3], true).is_empty());
        // Aynı adres iki kez: iki FARKLI kopya; büyükten küçüğe.
        assert_eq!(
            resolve_rows(&state, &[track(1), track(1)], &[], false),
            [(2, track(1)), (0, track(1))]
        );
        // Listede olmayan atlanır.
        assert!(resolve_rows(&state, &[track(9)], &[], false).is_empty());
        // Çok şarkı: büyük sıradan küçüğe.
        assert_eq!(
            resolve_rows(&state, &[track(2), track(3)], &[], false),
            [(3, track(3)), (1, track(2))]
        );

        let c = roundtrip(&changes_remove_rows(b"r", &[(4, track(1)), (0, track(1))]));
        let ops = &c.deltas[0].ops;
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].kind(), Kind::REM);
        assert_eq!((ops[0].rem.from_index(), ops[0].rem.length()), (4, 1));
        assert!(!ops[0].rem.items_as_key());
        assert_eq!(ops[1].rem.from_index(), 0);
    }

    #[test]
    fn add_at_position_and_fill_changes() {
        let c = roundtrip(&changes_add(b"r", &[track(1)], AddPosition::At(1)));
        let add = &c.deltas[0].ops[0].add;
        assert_eq!(add.from_index(), 1);
        assert!(!add.add_first() && !add.add_last());
        // Eski JSON kaydı ("last") ve yeni biçim ({"at":n}) birlikte okunur.
        assert_eq!(
            serde_json::from_str::<AddPosition>("\"last\"").unwrap(),
            AddPosition::Last
        );
        assert_eq!(
            serde_json::from_str::<AddPosition>("{\"at\":3}").unwrap(),
            AddPosition::At(3)
        );

        // Yeni liste: ad + şarkılar tek delta'da; yazılacak şey yoksa None.
        let fill = roundtrip(&changes_fill(b"r", Some("Ad"), &[track(1), track(2)]).unwrap());
        assert_eq!(fill.deltas.len(), 1);
        assert_eq!(fill.deltas[0].ops[0].kind(), Kind::UPDATE_LIST_ATTRIBUTES);
        assert_eq!(fill.deltas[0].ops[1].kind(), Kind::ADD);
        assert!(changes_fill(b"r", None, &[]).is_none());
        assert_eq!(
            changes_fill(b"r", None, &[track(1)]).unwrap().deltas[0]
                .ops
                .len(),
            1
        );

        // Tek satır çıkarmanın tersi eski yerine ekler.
        let rem = EditOp::RemoveFromPlaylist {
            playlist: playlist(1),
            uris: vec![track(1)],
            indices: vec![4],
        };
        let EditOp::AddToPlaylist { position, .. } = rem.inverse().unwrap() else {
            panic!()
        };
        assert_eq!(position, AddPosition::At(4));
    }

    #[test]
    fn move_applied_detection() {
        let order: Vec<String> = (0..5).map(track).collect();
        assert!(move_applied(&order, 2, &[track(2), track(3)]));
        assert!(!move_applied(&order, 1, &[track(2), track(3)]));
        assert!(!move_applied(&order, 4, &[track(4), track(5)]));
        assert!(!move_applied(&order, 0, &[]));
    }

    // ── İşlem özellikleri ───────────────────────────────────────────

    #[test]
    fn inverse_ops_and_validation() {
        let save = EditOp::Save { uri: track(1) };
        assert_eq!(save.inverse().unwrap(), EditOp::Unsave { uri: track(1) });
        assert_eq!(save.inverse().unwrap().inverse().unwrap(), save);
        let rename = EditOp::RenamePlaylist {
            playlist: playlist(1),
            name: "B".into(),
            previous_name: Some("A".into()),
        };
        let EditOp::RenamePlaylist {
            name,
            previous_name,
            ..
        } = rename.inverse().unwrap()
        else {
            panic!()
        };
        assert_eq!((name.as_str(), previous_name.as_deref()), ("A", Some("B")));
        // Eski ad bilinmiyorsa geri alınamaz.
        assert!(
            EditOp::RenamePlaylist {
                playlist: playlist(1),
                name: "B".into(),
                previous_name: None
            }
            .inverse()
            .is_none()
        );
        assert!(
            EditOp::CreatePlaylist {
                temp_id: new_temp_id(),
                name: "x".into(),
                uris: vec![],
                created: None
            }
            .inverse()
            .is_none()
        );
        // Doğrulama.
        assert!(
            EditOp::Save {
                uri: "spotify:artist:x".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            EditOp::AddToPlaylist {
                playlist: playlist(1),
                uris: vec![],
                position: AddPosition::Last
            }
            .validate()
            .is_err()
        );
        assert!(
            EditOp::AddToPlaylist {
                playlist: playlist(1),
                uris: vec!["spotify:episode:e".into()],
                position: AddPosition::Last
            }
            .validate()
            .is_err()
        );
        assert!(
            EditOp::RenamePlaylist {
                playlist: playlist(1),
                name: "  ".into(),
                previous_name: None
            }
            .validate()
            .is_err()
        );
        assert!(
            EditOp::MovePlaylistItems {
                playlist: playlist(1),
                from_index: 2,
                length: 1,
                to_index: 2,
                uris: vec![]
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn ops_roundtrip_through_json() {
        let ops = vec![
            EditOp::Save { uri: track(1) },
            EditOp::AddToPlaylist {
                playlist: playlist(1),
                uris: vec![track(1)],
                position: AddPosition::First,
            },
            EditOp::CreatePlaylist {
                temp_id: "local:playlist:1".into(),
                name: "A".into(),
                uris: vec![],
                created: Some(playlist(9)),
            },
            EditOp::MovePlaylistItems {
                playlist: playlist(1),
                from_index: 1,
                length: 2,
                to_index: 4,
                uris: vec![track(1), track(2)],
            },
            EditOp::DeletePlaylist {
                playlist: playlist(2),
            },
        ];
        for op in ops {
            let json = serde_json::to_string(&op).unwrap();
            assert_eq!(serde_json::from_str::<EditOp>(&json).unwrap(), op);
        }
        // Eski kayıtta olmayan isteğe bağlı alanlar varsayılanla okunur.
        let old = r#"{"op":"create_playlist","temp_id":"local:playlist:1","name":"A","uris":[]}"#;
        assert!(matches!(
            serde_json::from_str::<EditOp>(old).unwrap(),
            EditOp::CreatePlaylist { created: None, .. }
        ));
    }

    #[test]
    fn error_classification_and_policy() {
        use librespot::core::Error;
        let c = |e: Error| EditError::from_spotify(&e);
        assert!(matches!(c(Error::unavailable("x")), EditError::Offline(_)));
        assert!(matches!(
            c(Error::deadline_exceeded("x")),
            EditError::Offline(_)
        ));
        assert!(matches!(
            c(Error::resource_exhausted("x")),
            EditError::RateLimited { .. }
        ));
        assert!(matches!(
            c(Error::unauthenticated("x")),
            EditError::Unauthorized(_)
        ));
        assert!(matches!(
            c(Error::permission_denied("x")),
            EditError::Forbidden(_)
        ));
        assert!(matches!(c(Error::not_found("x")), EditError::NotFound(_)));
        assert!(matches!(
            c(Error::failed_precondition("x")),
            EditError::Conflict(_)
        ));
        assert!(matches!(
            c(Error::invalid_argument("x")),
            EditError::Invalid(_)
        ));
        assert!(matches!(
            c(Error::unimplemented("x")),
            EditError::Unsupported(_)
        ));
        assert!(matches!(c(Error::unknown("x")), EditError::Server(_)));

        // HTTP durum kodu olan `Unavailable` (500 / 503 / 421) sunucu hatasıdır,
        // çevrimdışı değil; bağlantı hatası (durum kodu yok) çevrimdışı kalır.
        use http::StatusCode;
        for code in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::MISDIRECTED_REQUEST,
        ] {
            let e = c(Error::unavailable(format!("Response status code: {code}")));
            assert!(matches!(e, EditError::Server(_)), "{code}: {e:?}");
            assert!(!e.is_offline());
            assert!(e.is_retryable(SERVER_ERROR_ATTEMPTS - 1));
            assert!(!e.is_retryable(SERVER_ERROR_ATTEMPTS));
        }
        let conn = c(Error::unavailable("client error (Connect): dns error"));
        assert!(matches!(conn, EditError::Offline(_)));
        assert!(conn.is_offline() && conn.is_retryable(1000));

        // Süresiz: ağ / yavaşla / oturum. Sınırlı: sunucu. Kalıcı: izin, yok, geçersiz.
        assert!(EditError::Offline("x".into()).is_retryable(1000));
        assert!(EditError::Server("x".into()).is_retryable(SERVER_ERROR_ATTEMPTS - 1));
        assert!(!EditError::Server("x".into()).is_retryable(SERVER_ERROR_ATTEMPTS));
        assert!(!EditError::Forbidden("x".into()).is_retryable(1));
        assert!(!EditError::Invalid("x".into()).is_retryable(1));
        // Yarım oluşturma, içindeki hatanın politikasını izler.
        let partial = EditError::PartialCreate {
            created_uri: playlist(3),
            source: Box::new(EditError::Offline("x".into())),
        };
        assert!(partial.is_retryable(50));
        assert_eq!(partial.partial_create(), Some(playlist(3).as_str()));
        assert!(partial.may_have_applied());
        // Gecikme: 5, 15, 45 sn ... tavan 15 dk.
        let off = EditError::Offline("x".into());
        assert_eq!(backoff_ms(1, &off), 5_000);
        assert_eq!(backoff_ms(2, &off), 15_000);
        assert_eq!(backoff_ms(3, &off), 45_000);
        assert_eq!(backoff_ms(50, &off), BACKOFF_MAX_MS);
        let limited = EditError::RateLimited {
            retry_after_ms: Some(120_000),
        };
        assert_eq!(backoff_ms(1, &limited), 120_000);
    }

    // ── Kuyruk ──────────────────────────────────────────────────────

    /// Sıradaki sonuçları veren sahte arka uç; uygulanan işlemleri kaydeder.
    struct Fake {
        results: Mutex<VecDeque<Result<EditOutcome, EditError>>>,
        applied: Mutex<Vec<(EditOp, bool)>>,
    }

    impl Fake {
        fn new(results: Vec<Result<EditOutcome, EditError>>) -> Self {
            Self {
                results: Mutex::new(results.into()),
                applied: Mutex::new(Vec::new()),
            }
        }
        fn applied(&self) -> Vec<EditOp> {
            self.applied
                .lock()
                .unwrap()
                .iter()
                .map(|(o, _)| o.clone())
                .collect()
        }
    }

    impl EditBackend for Fake {
        async fn apply(&self, op: &EditOp, ctx: ApplyCtx) -> Result<EditOutcome, EditError> {
            self.applied
                .lock()
                .unwrap()
                .push((op.clone(), ctx.maybe_applied));
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(EditOutcome::Done))
        }
    }

    fn offline() -> EditError {
        EditError::Offline("ağ yok".into())
    }

    #[test]
    fn like_unlike_cancel_out_but_only_when_unsent() {
        let q = EditQueue::open_in_memory().unwrap();
        let save = EditOp::Save { uri: track(1) };
        let unsave = EditOp::Unsave { uri: track(1) };
        assert!(matches!(
            q.enqueue_at(&save, 0).unwrap(),
            Enqueued::Queued(_)
        ));
        assert!(matches!(
            q.enqueue_at(&save, 0).unwrap(),
            Enqueued::Duplicate(_)
        ));
        assert_eq!(q.enqueue_at(&unsave, 0).unwrap(), Enqueued::Cancelled);
        assert!(q.pending().unwrap().is_empty());
        // Başka şarkı etkilenmez.
        q.enqueue_at(&save, 0).unwrap();
        q.enqueue_at(&EditOp::Unsave { uri: track(2) }, 0).unwrap();
        assert_eq!(q.pending().unwrap().len(), 2);
    }

    // ── Sanatçı takibi ──────────────────────────────────────────────

    fn artist(n: u32) -> String {
        format!("spotify:artist:{n:022}")
    }

    /// Yazma gövdesinden (küme, adresler, silindi mi).
    fn decode_write(body: &[u8]) -> (String, Vec<(String, bool)>) {
        let mut is = protobuf::CodedInputStream::from_bytes(body);
        let (mut set, mut items) = (String::new(), Vec::new());
        while let Some(tag) = is.read_raw_tag_or_eof().unwrap() {
            match tag >> 3 {
                1 | 4 => {
                    is.read_string().unwrap();
                }
                2 => set = is.read_string().unwrap(),
                3 => {
                    let raw = is.read_bytes().unwrap();
                    let mut it = protobuf::CodedInputStream::from_bytes(&raw);
                    let (mut uri, mut removed) = (String::new(), false);
                    while let Some(t) = it.read_raw_tag_or_eof().unwrap() {
                        match t >> 3 {
                            1 => uri = it.read_string().unwrap(),
                            2 => {
                                it.read_int32().unwrap();
                            }
                            3 => removed = it.read_bool().unwrap(),
                            _ => panic!("beklenmeyen alan"),
                        }
                    }
                    items.push((uri, removed));
                }
                _ => panic!("beklenmeyen alan"),
            }
        }
        (set, items)
    }

    #[test]
    fn follow_writes_go_to_the_artist_set_and_tracks_stay_in_collection() {
        let follow =
            collection_write_body_in(ARTIST_SET, "k", &[artist(1)], false, 5, "u").unwrap();
        assert_eq!(
            decode_write(&follow),
            ("artist".to_string(), vec![(artist(1), false)])
        );
        let unfollow =
            collection_write_body_in(ARTIST_SET, "k", &[artist(1)], true, 5, "u").unwrap();
        assert_eq!(
            decode_write(&unfollow),
            ("artist".to_string(), vec![(artist(1), true)])
        );
        // Şarkı / albüm yazması değişmedi: hâlâ "collection".
        let like = collection_write_body("k", &[track(1)], false, 5, "u").unwrap();
        assert_eq!(decode_write(&like).0, "collection");
    }

    #[test]
    fn follow_validation_inverse_and_json() {
        let follow = EditOp::Follow { uri: artist(1) };
        assert_eq!(
            follow.inverse().unwrap(),
            EditOp::Unfollow { uri: artist(1) }
        );
        assert_eq!(follow.inverse().unwrap().inverse().unwrap(), follow);
        assert!(follow.validate().is_ok());
        // Yalnız sanatçı takip edilir; şarkı / albüm / boş / bozuk kimlik reddedilir.
        for bad in [
            track(1),
            "spotify:album:x".to_string(),
            "spotify:artist:".to_string(),
            "spotify:artist:a b".to_string(),
            String::new(),
        ] {
            assert!(
                EditOp::Follow { uri: bad.clone() }.validate().is_err(),
                "{bad:?}"
            );
            assert!(EditOp::Unfollow { uri: bad }.validate().is_err());
        }
        // Takip, şarkı / albüm kaydetmenin yerine geçmez.
        assert!(EditOp::Save { uri: artist(1) }.validate().is_err());
        // Kuyruktan sonra da aynı işlem okunur; hedefi beğeni hedefinden ayrı.
        for op in [follow.clone(), EditOp::Unfollow { uri: artist(2) }] {
            let json = serde_json::to_string(&op).unwrap();
            assert_eq!(serde_json::from_str::<EditOp>(&json).unwrap(), op);
        }
        assert!(
            serde_json::to_string(&follow)
                .unwrap()
                .contains("\"op\":\"follow\"")
        );
        assert_ne!(follow.target(), EditOp::Save { uri: track(1) }.target());
        assert_eq!(
            follow.target(),
            EditOp::Unfollow { uri: artist(9) }.target()
        );
    }

    #[test]
    fn follow_unfollow_cancel_out_only_when_unsent() {
        let q = EditQueue::open_in_memory().unwrap();
        let follow = EditOp::Follow { uri: artist(1) };
        let unfollow = EditOp::Unfollow { uri: artist(1) };
        assert!(matches!(
            q.enqueue_at(&follow, 0).unwrap(),
            Enqueued::Queued(_)
        ));
        assert!(matches!(
            q.enqueue_at(&follow, 0).unwrap(),
            Enqueued::Duplicate(_)
        ));
        // Takip + takipten çık (geri al): Spotify'a hiçbir şey gitmez.
        assert_eq!(q.enqueue_at(&unfollow, 0).unwrap(), Enqueued::Cancelled);
        assert!(q.pending().unwrap().is_empty());
        // Başka sanatçı etkilenmez; takip, aynı adresli şarkı beğenisiyle karışmaz.
        q.enqueue_at(&follow, 0).unwrap();
        q.enqueue_at(&EditOp::Unfollow { uri: artist(2) }, 0)
            .unwrap();
        assert_eq!(q.pending().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn follow_waits_offline_retries_with_the_uncertain_flag_and_can_be_undone() {
        let q = EditQueue::open_in_memory().unwrap();
        let follow = EditOp::Follow { uri: artist(1) };
        q.enqueue_at(&follow, 0).unwrap();
        // Ağ yok: kayıtlı kalır, hata türü saklanır, yeniden deneme zamanı konur.
        let fake = Fake::new(vec![Err(offline())]);
        let report = q.process_due(&fake, 1_000).await.unwrap();
        assert!(report.offline && report.done.is_empty());
        let waiting = q.pending().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!((waiting[0].attempts, waiting[0].maybe_applied), (1, true));
        assert_eq!(waiting[0].error_kind, Some(FailKind::Offline));
        // Zamanı gelmeden ağa gidilmez.
        let early = Fake::new(vec![]);
        q.process_due(&early, 2_000).await.unwrap();
        assert!(early.applied().is_empty());
        // Ağ geldi: aynı işlem, "belirsiz" bayrağıyla yeniden gider ve kuyruktan düşer.
        let again = Fake::new(vec![]);
        let report = q
            .process_due(&again, 1_000 + BACKOFF_BASE_MS + 1)
            .await
            .unwrap();
        assert_eq!(report.done_ops, vec![follow.clone()]);
        assert_eq!(again.applied.lock().unwrap()[0], (follow.clone(), true));
        assert!(q.pending().unwrap().is_empty() && q.failed().unwrap().is_empty());
        // Geri al (Ctrl+Z): ters işlem normal sıraya girer ve yazılır.
        let undo = follow.inverse().unwrap();
        assert!(matches!(
            q.enqueue_at(&undo, 10_000_000).unwrap(),
            Enqueued::Queued(_)
        ));
        let last = Fake::new(vec![]);
        q.process_due(&last, 10_000_001).await.unwrap();
        assert_eq!(last.applied(), vec![EditOp::Unfollow { uri: artist(1) }]);
    }

    #[tokio::test]
    async fn refused_follow_stays_visible_with_its_kind_and_does_not_block_likes() {
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(&EditOp::Follow { uri: artist(1) }, 0).unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
        let fake = Fake::new(vec![Err(EditError::NotFound("sanatçı yok".into()))]);
        let report = q.process_due(&fake, 10).await.unwrap();
        // Takip kalıcı olarak reddedildi (görünür kalır); beğeni ayrı hedef, yazıldı.
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.done_ops, vec![EditOp::Save { uri: track(1) }]);
        let failed = q.failed().unwrap();
        assert_eq!(failed[0].op, EditOp::Follow { uri: artist(1) });
        assert_eq!(failed[0].error_kind, Some(FailKind::NotFound));
        // "Tekrar dene" sıraya döndürür ve yazılır.
        q.retry(failed[0].id).unwrap();
        let ok = Fake::new(vec![]);
        assert_eq!(q.process_due(&ok, 20).await.unwrap().done.len(), 1);
    }

    #[test]
    fn enqueue_rejects_invalid_ops_without_writing() {
        let q = EditQueue::open_in_memory().unwrap();
        assert!(
            q.enqueue(&EditOp::Save {
                uri: "spotify:artist:x".into()
            })
            .is_err()
        );
        assert!(q.pending().unwrap().is_empty());
    }

    #[tokio::test]
    async fn successful_edits_are_removed_in_order() {
        let q = EditQueue::open_in_memory().unwrap();
        for n in 1..=3 {
            q.enqueue_at(
                &EditOp::AddToPlaylist {
                    playlist: playlist(1),
                    uris: vec![track(n)],
                    position: AddPosition::Last,
                },
                0,
            )
            .unwrap();
        }
        let fake = Fake::new(vec![]);
        let report = q.process_due(&fake, 1000).await.unwrap();
        assert_eq!(report.done.len(), 3);
        assert!(q.pending().unwrap().is_empty() && q.failed().unwrap().is_empty());
        let order: Vec<_> = fake
            .applied()
            .into_iter()
            .map(|o| match o {
                EditOp::AddToPlaylist { uris, .. } => uris[0].clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(order, [track(1), track(2), track(3)]);
    }

    #[tokio::test]
    async fn offline_keeps_everything_and_stops_early() {
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
        q.enqueue_at(
            &EditOp::RenamePlaylist {
                playlist: playlist(1),
                name: "A".into(),
                previous_name: None,
            },
            0,
        )
        .unwrap();
        let fake = Fake::new(vec![Err(offline())]);
        let report = q.process_due(&fake, 1000).await.unwrap();
        assert!(report.offline);
        // Ağ yok: ikinci işleme hiç denenmedi, ikisi de kayıtlı.
        assert_eq!(fake.applied().len(), 1);
        assert_eq!(q.pending().unwrap().len(), 2);
        let first = &q.pending().unwrap()[0];
        assert_eq!(first.attempts, 1);
        assert!(first.maybe_applied);
        assert!(first.last_error.as_deref().unwrap().contains("ağ yok"));
        assert_eq!(report.next_due_ms, Some(1000 + 5_000));
        // Geri çekilen işlemin zamanı henüz gelmedi: ona ağa gidilmez; hiç
        // denenmemiş olan (başka hedef) ise sürer.
        let fake2 = Fake::new(vec![]);
        let again = q.process_due(&fake2, 2000).await.unwrap();
        assert_eq!(fake2.applied().len(), 1);
        assert!(matches!(fake2.applied()[0], EditOp::RenamePlaylist { .. }));
        assert_eq!(again.done.len(), 1);
        assert_eq!(again.next_due_ms, Some(6000));
        // Zamanı gelince sürer; maybe_applied arka uca iletilir.
        let report = q.process_due(&fake2, 7000).await.unwrap();
        assert_eq!(report.done.len(), 1);
        assert!(fake2.applied.lock().unwrap()[1].1);
        assert!(q.pending().unwrap().is_empty());
    }

    #[tokio::test]
    async fn retry_survives_restart() {
        let path = std::env::temp_dir().join(format!("openify-edits-{}.db", now_ms()));
        {
            let q = EditQueue::open(&path).unwrap();
            q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
            let fake = Fake::new(vec![Err(offline())]);
            q.process_due(&fake, 0).await.unwrap();
        }
        // "Uygulama kapandı ve yeniden açıldı."
        let q = EditQueue::open(&path).unwrap();
        let pending = q.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 1);
        let fake = Fake::new(vec![]);
        let report = q.process_due(&fake, 100_000).await.unwrap();
        assert_eq!(report.done.len(), 1);
        drop(q);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[tokio::test]
    async fn crash_during_flight_marks_maybe_applied() {
        let path = std::env::temp_dir().join(format!("openify-edits-crash-{}.db", now_ms()));
        {
            let q = EditQueue::open(&path).unwrap();
            let Enqueued::Queued(id) = q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap()
            else {
                panic!()
            };
            q.set_in_flight(id).unwrap();
            // Çökme: yanıt hiç işlenmedi.
        }
        let q = EditQueue::open(&path).unwrap();
        let edit = &q.pending().unwrap()[0];
        assert!(edit.maybe_applied, "belirsiz diye işaretlenmeli");
        // Belirsiz işlem birleştirme/iptale girmez (sunucuda olabilir).
        assert!(matches!(
            q.enqueue_at(&EditOp::Unsave { uri: track(1) }, 1).unwrap(),
            Enqueued::Queued(_)
        ));
        drop(q);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[tokio::test]
    async fn permanent_failure_is_kept_visible_not_dropped() {
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(
            &EditOp::RenamePlaylist {
                playlist: playlist(1),
                name: "A".into(),
                previous_name: None,
            },
            0,
        )
        .unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
        let fake = Fake::new(vec![Err(EditError::Forbidden("senin değil".into()))]);
        let report = q.process_due(&fake, 10).await.unwrap();
        assert_eq!(report.failed.len(), 1);
        // Başarısız olan kaydı duruyor, gerekçesiyle; sonraki farklı hedef yazıldı.
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0]
                .last_error
                .as_deref()
                .unwrap()
                .contains("senin değil")
        );
        assert_eq!(report.done.len(), 1);
        // Tekrar işlenince yeniden denenmez (kalıcı), ama silinmez.
        let fake2 = Fake::new(vec![]);
        q.process_due(&fake2, 20).await.unwrap();
        assert!(fake2.applied().is_empty());
        assert_eq!(q.failed().unwrap().len(), 1);
        // Kullanıcı "tekrar dene" derse sıraya döner ve yazılır.
        q.retry(failed[0].id).unwrap();
        let report = q.process_due(&fake2, 30).await.unwrap();
        assert_eq!(report.done.len(), 1);
        assert!(q.failed().unwrap().is_empty());
        // "Bırak" yalnız başarısızı siler, bekleyeni silmez.
        q.enqueue_at(&EditOp::Save { uri: track(9) }, 40).unwrap();
        let pending_id = q.pending().unwrap()[0].id;
        q.dismiss_cascade(pending_id).unwrap();
        assert_eq!(q.pending().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn blocked_target_keeps_order_but_others_proceed() {
        let q = EditQueue::open_in_memory().unwrap();
        let add = |p: u32, n: u32| EditOp::AddToPlaylist {
            playlist: playlist(p),
            uris: vec![track(n)],
            position: AddPosition::Last,
        };
        q.enqueue_at(&add(1, 1), 0).unwrap(); // sunucu hatası alacak
        q.enqueue_at(&add(1, 2), 0).unwrap(); // aynı liste: beklemeli
        q.enqueue_at(&add(2, 3), 0).unwrap(); // başka liste: gitmeli
        let fake = Fake::new(vec![Err(EditError::Server("500".into()))]);
        let report = q.process_due(&fake, 0).await.unwrap();
        assert!(!report.offline);
        assert_eq!(report.retry_later.len(), 1);
        assert_eq!(report.done.len(), 1);
        let applied = fake.applied();
        assert_eq!(applied.len(), 2);
        assert!(applied.contains(&add(1, 1)) && applied.contains(&add(2, 3)));
        assert!(!applied.contains(&add(1, 2)));
        assert_eq!(q.pending().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn server_errors_give_up_after_budget_and_stay_visible() {
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
        let fake = Fake::new(
            (0..20)
                .map(|_| Err(EditError::Server("500".into())))
                .collect(),
        );
        let mut now = 0;
        for _ in 0..SERVER_ERROR_ATTEMPTS {
            now += BACKOFF_MAX_MS + 1;
            q.process_due(&fake, now).await.unwrap();
        }
        assert_eq!(fake.applied().len(), SERVER_ERROR_ATTEMPTS as usize);
        assert!(q.pending().unwrap().is_empty());
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].attempts, SERVER_ERROR_ATTEMPTS);
    }

    #[tokio::test]
    async fn too_old_edit_is_reported_not_silently_dropped() {
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
        let fake = Fake::new(vec![]);
        let report = q.process_due(&fake, MAX_AGE_MS + 1).await.unwrap();
        assert_eq!(report.failed.len(), 1);
        assert!(fake.applied().is_empty());
        assert!(
            q.failed().unwrap()[0]
                .last_error
                .as_deref()
                .unwrap()
                .contains("gündür")
        );
    }

    #[tokio::test]
    async fn offline_create_then_edit_rewrites_to_real_uri() {
        let q = EditQueue::open_in_memory().unwrap();
        let temp = new_temp_id();
        q.enqueue_at(
            &EditOp::CreatePlaylist {
                temp_id: temp.clone(),
                name: "Yeni".into(),
                uris: vec![],
                created: None,
            },
            0,
        )
        .unwrap();
        // Liste henüz Spotify'da yokken ona ekleme ve ad değiştirme.
        q.enqueue_at(
            &EditOp::AddToPlaylist {
                playlist: temp.clone(),
                uris: vec![track(1)],
                position: AddPosition::Last,
            },
            0,
        )
        .unwrap();
        q.enqueue_at(
            &EditOp::RenamePlaylist {
                playlist: temp.clone(),
                name: "Yeni 2".into(),
                previous_name: None,
            },
            0,
        )
        .unwrap();
        // Önce ağ yok: üçü de sırada, sıra korunur.
        let fake = Fake::new(vec![Err(offline())]);
        q.process_due(&fake, 0).await.unwrap();
        assert_eq!(q.pending().unwrap().len(), 3);
        // Sonra ağ geldi: oluşturma gerçek adresi verir, ardıllar ona yönlenir.
        let real = playlist(77);
        let fake = Fake::new(vec![Ok(EditOutcome::Created { uri: real.clone() })]);
        let report = q.process_due(&fake, 10_000).await.unwrap();
        assert_eq!(report.done.len(), 3);
        let applied = fake.applied();
        assert!(
            matches!(applied[1], EditOp::AddToPlaylist { ref playlist, .. } if *playlist == real)
        );
        assert!(
            matches!(applied[2], EditOp::RenamePlaylist { ref playlist, .. } if *playlist == real)
        );
        assert!(q.pending().unwrap().is_empty());
    }

    #[tokio::test]
    async fn partial_create_remembers_created_uri_to_avoid_duplicates() {
        let q = EditQueue::open_in_memory().unwrap();
        let temp = new_temp_id();
        q.enqueue_at(
            &EditOp::CreatePlaylist {
                temp_id: temp.clone(),
                name: "Yeni".into(),
                uris: vec![],
                created: None,
            },
            0,
        )
        .unwrap();
        let real = playlist(5);
        let fake = Fake::new(vec![Err(EditError::PartialCreate {
            created_uri: real.clone(),
            source: Box::new(EditError::Server("kök liste".into())),
        })]);
        q.process_due(&fake, 0).await.unwrap();
        let edit = &q.pending().unwrap()[0];
        assert!(matches!(&edit.op, EditOp::CreatePlaylist { created: Some(c), .. } if *c == real));
        // Yeniden denemede arka uç `created`'i görür (yeni liste açmaz).
        let fake2 = Fake::new(vec![Ok(EditOutcome::Created { uri: real.clone() })]);
        q.process_due(&fake2, BACKOFF_MAX_MS + 1).await.unwrap();
        assert!(matches!(
            &fake2.applied()[0],
            EditOp::CreatePlaylist { created: Some(c), .. } if *c == real
        ));
    }

    #[tokio::test]
    async fn failed_create_fails_dependents_visibly() {
        let q = EditQueue::open_in_memory().unwrap();
        let temp = new_temp_id();
        q.enqueue_at(
            &EditOp::CreatePlaylist {
                temp_id: temp.clone(),
                name: "Yeni".into(),
                uris: vec![],
                created: None,
            },
            0,
        )
        .unwrap();
        q.enqueue_at(
            &EditOp::AddToPlaylist {
                playlist: temp,
                uris: vec![track(1)],
                position: AddPosition::Last,
            },
            0,
        )
        .unwrap();
        let fake = Fake::new(vec![Err(EditError::Forbidden("izin yok".into()))]);
        let report = q.process_due(&fake, 0).await.unwrap();
        assert_eq!(report.failed.len(), 2);
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 2);
        assert!(
            failed[1]
                .last_error
                .as_deref()
                .unwrap()
                .contains("oluşturulamadı")
        );
        assert!(q.pending().unwrap().is_empty());
        // Türleri: oluşturma izin hatası, ardıl "bağlı olduğu liste" hatası.
        assert_eq!(failed[0].error_kind, Some(FailKind::Forbidden));
        assert_eq!(failed[1].error_kind, Some(FailKind::ParentFailed));
    }

    // ── Hata türü (Adım 13 arayüz düzeltmesi) ───────────────────────

    #[test]
    fn error_kinds_map_one_to_one_and_survive_the_tag_roundtrip() {
        let cases: Vec<(EditError, FailKind)> = vec![
            (EditError::Offline("x".into()), FailKind::Offline),
            (
                EditError::RateLimited {
                    retry_after_ms: None,
                },
                FailKind::RateLimited,
            ),
            (EditError::Unauthorized("x".into()), FailKind::Unauthorized),
            (EditError::Forbidden("x".into()), FailKind::Forbidden),
            (EditError::NotFound("x".into()), FailKind::NotFound),
            (EditError::Conflict("x".into()), FailKind::Conflict),
            (EditError::Invalid("x".into()), FailKind::Invalid),
            (EditError::Unsupported("x".into()), FailKind::Unsupported),
            (EditError::Server("x".into()), FailKind::Server),
            (EditError::Expired("x".into()), FailKind::Expired),
            // Yarım oluşturma, içindeki hatanın türünü taşır.
            (
                EditError::PartialCreate {
                    created_uri: playlist(1),
                    source: Box::new(EditError::NotFound("x".into())),
                },
                FailKind::NotFound,
            ),
        ];
        for (error, kind) in cases {
            assert_eq!(error.kind(), kind, "{error}");
            assert_eq!(FailKind::from_tag(kind.tag()), kind);
        }
        // Tanınmayan etiket (eski / bozuk kayıt) "bilinmiyor" olur.
        assert_eq!(FailKind::from_tag("saçma"), FailKind::Unknown);
    }

    #[tokio::test]
    async fn failure_kind_comes_from_the_error_not_from_its_text() {
        // Metinde başka bir türün anahtar sözcüğü geçse de tür hatanın kendisinden gelir
        // (eski arayüz metne bakıp "reddedildi" geçen her hatayı "izin yok" sanıyordu).
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(&EditOp::Unsave { uri: track(1) }, 0).unwrap();
        q.enqueue_at(
            &EditOp::RenamePlaylist {
                playlist: playlist(1),
                name: "A".into(),
                previous_name: None,
            },
            0,
        )
        .unwrap();
        let fake = Fake::new(vec![
            Err(EditError::NotFound("izin yok, reddedildi".into())),
            Err(EditError::Forbidden("bulunamadı".into())),
        ]);
        q.process_due(&fake, 0).await.unwrap();
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[0].error_kind, Some(FailKind::NotFound));
        assert_eq!(failed[1].error_kind, Some(FailKind::Forbidden));
        // Yeniden denenecek hata da türünü taşır; "tekrar dene" eski hatayı siler.
        q.enqueue_at(&EditOp::Save { uri: track(2) }, 0).unwrap();
        let fake = Fake::new(vec![Err(EditError::Server("500".into()))]);
        q.process_due(&fake, 10).await.unwrap();
        let retrying = &q.pending().unwrap()[0];
        assert_eq!(retrying.error_kind, Some(FailKind::Server));
        q.retry(failed[0].id).unwrap();
        let again = q.get(failed[0].id).unwrap().unwrap();
        assert_eq!(
            (again.state, again.error_kind, again.last_error),
            (EditState::Pending, None, None)
        );
    }

    #[tokio::test]
    async fn giving_up_after_days_is_its_own_kind() {
        let q = EditQueue::open_in_memory().unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(1) }, 0).unwrap();
        q.process_due(&Fake::new(vec![]), MAX_AGE_MS + 1)
            .await
            .unwrap();
        assert_eq!(q.failed().unwrap()[0].error_kind, Some(FailKind::Expired));
    }

    #[test]
    fn database_from_before_error_kind_is_migrated_in_place() {
        let path = std::env::temp_dir().join(format!("openify-edits-migrate-{}.db", now_ms()));
        {
            // Sütunsuz eski şema + başarısız bir kayıt.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE edits (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, op TEXT NOT NULL, target TEXT NOT NULL,
                     state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
                     next_attempt_ms INTEGER NOT NULL DEFAULT 0, last_error TEXT,
                     maybe_applied INTEGER NOT NULL DEFAULT 0, in_flight INTEGER NOT NULL DEFAULT 0,
                     created_ms INTEGER NOT NULL);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO edits (op, target, state, last_error, created_ms)
                 VALUES (?1, 'collection', 'failed', 'Bu işleme izin yok (liste senin değil mi?): 403', 5)",
                [serde_json::to_string(&EditOp::Unsave { uri: track(1) }).unwrap()],
            )
            .unwrap();
        }
        let q = EditQueue::open(&path).unwrap();
        let failed = q.failed().unwrap();
        assert_eq!(failed.len(), 1);
        // Eski kayıt: tür saklanmamış, metni korunmuş; kuyruk çalışıyor.
        assert_eq!(failed[0].error_kind, None);
        assert!(
            failed[0]
                .last_error
                .as_deref()
                .unwrap()
                .contains("izin yok")
        );
        q.enqueue_at(&EditOp::Save { uri: track(2) }, 6).unwrap();
        assert_eq!(q.pending().unwrap().len(), 1);
        // İkinci açılış sütunu yeniden eklemeye çalışmaz.
        drop(q);
        assert!(EditQueue::open(&path).is_ok());
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    // ── Şema göçü: kullanıcı verisi (bekleyen yazmalar) hiçbir yoldan kaybolmaz ──

    /// Göçten önce (ya da sütun eklenmeden önce) yazılmış kuyruk dosyası: bir
    /// bekleyen, bir gönderimde kalmış (sonucu belirsiz), bir başarısız kayıt.
    fn legacy_queue_file(path: &Path, with_kind: bool, version: i64) {
        let conn = Connection::open(path).unwrap();
        let kind = if with_kind { ", error_kind TEXT" } else { "" };
        conn.execute_batch(&format!(
            "CREATE TABLE edits (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, op TEXT NOT NULL, target TEXT NOT NULL,
                 state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
                 next_attempt_ms INTEGER NOT NULL DEFAULT 0, last_error TEXT,
                 maybe_applied INTEGER NOT NULL DEFAULT 0, in_flight INTEGER NOT NULL DEFAULT 0,
                 created_ms INTEGER NOT NULL{kind});
             PRAGMA user_version = {version};"
        ))
        .unwrap();
        let op = |uri| serde_json::to_string(&EditOp::Save { uri }).unwrap();
        for (op, state, attempts, in_flight, err, created) in [
            (op(track(1)), "pending", 2, 0, None, 10),
            (op(track(2)), "pending", 1, 1, None, 20),
            (op(track(3)), "failed", 6, 0, Some("sunucu 500"), 30),
        ] {
            conn.execute(
                "INSERT INTO edits (op, target, state, attempts, in_flight, last_error, created_ms)
                 VALUES (?1, 'collection', ?2, ?3, ?4, ?5, ?6)",
                params![op, state, attempts, in_flight, err, created],
            )
            .unwrap();
        }
    }

    #[test]
    fn every_older_queue_version_reaches_the_latest_and_loses_no_edit() {
        let dir = std::env::temp_dir().join(format!("openify-edits-versions-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = EditQueue::open(&dir.join("fresh.db")).unwrap();
        assert_eq!(migrate::user_version(&fresh.conn()).unwrap(), 2);
        let want = migrate::schema_columns(&fresh.conn());
        assert!(want["edits"].contains(&"error_kind".to_string()));
        drop(fresh);

        // (sürümlenmemiş sütunsuz, sürümlenmemiş sütunlu, 1, 2)
        for (tag, with_kind, version) in [
            ("v0", false, 0),
            ("v0-sutunlu", true, 0),
            ("v1", false, 1),
            ("v2", true, 2),
        ] {
            let path = dir.join(format!("{tag}.db"));
            legacy_queue_file(&path, with_kind, version);
            let q = EditQueue::open(&path).unwrap();
            assert_eq!(migrate::user_version(&q.conn()).unwrap(), 2, "{tag}");
            assert_eq!(migrate::schema_columns(&q.conn()), want, "{tag}: aynı şema");
            // Üç kayıt da yerinde, sırasıyla, içerikleri aynı.
            let pending = q.pending().unwrap();
            let failed = q.failed().unwrap();
            assert_eq!((pending.len(), failed.len()), (2, 1), "{tag}");
            assert_eq!(pending[0].op, EditOp::Save { uri: track(1) });
            assert_eq!(
                (
                    pending[0].attempts,
                    pending[0].created_ms,
                    pending[0].maybe_applied
                ),
                (2, 10, false)
            );
            // Gönderimde kapanan: sonucu belirsiz diye işaretlenir (kayıp değil).
            assert_eq!(pending[1].op, EditOp::Save { uri: track(2) });
            assert!(pending[1].maybe_applied && !pending[1].in_flight, "{tag}");
            assert_eq!(failed[0].op, EditOp::Save { uri: track(3) });
            assert_eq!(failed[0].last_error.as_deref(), Some("sunucu 500"));
            // Kuyruk çalışıyor: yeni işlem eklenebilir.
            q.enqueue_at(&EditOp::Save { uri: track(9) }, 40).unwrap();
            assert_eq!(q.pending().unwrap().len(), 3, "{tag}");
        }

        // Daha yeni uygulamanın yazdığı kuyruk açılmaz ve dokunulmaz.
        let path = dir.join("v2.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 5)
            .unwrap();
        let err = EditQueue::open(&path)
            .err()
            .expect("daha yeni sürüm açılmamalı");
        assert!(format!("{err:#}").contains("version 5"), "{err:#}");
        let raw = Connection::open(&path).unwrap();
        assert_eq!(migrate::user_version(&raw).unwrap(), 5);
        let n: i64 = raw
            .query_row("SELECT COUNT(*) FROM edits", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 4);
        drop(raw);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dismiss_only_drops_failed_and_cascades_from_a_failed_create() {
        let q = EditQueue::open_in_memory().unwrap();
        let temp = new_temp_id();
        q.enqueue_at(
            &EditOp::CreatePlaylist {
                temp_id: temp.clone(),
                name: "Yeni".into(),
                uris: vec![],
                created: None,
            },
            0,
        )
        .unwrap();
        q.enqueue_at(
            &EditOp::AddToPlaylist {
                playlist: temp.clone(),
                uris: vec![track(1)],
                position: AddPosition::Last,
            },
            0,
        )
        .unwrap();
        q.enqueue_at(&EditOp::Save { uri: track(5) }, 0).unwrap();
        q.process_due(&Fake::new(vec![Err(EditError::Forbidden("x".into()))]), 0)
            .await
            .unwrap();
        assert_eq!(q.failed().unwrap().len(), 2);
        // Oluşturma bırakılınca ona bağlı yazılamayan da gider; kuyruktaki
        // başka iş (başka hedef) ve yeniden denenenler kalır.
        let failed_create = q.failed().unwrap()[0].id;
        let removed = q.dismiss_cascade(failed_create).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(matches!(removed[0].op, EditOp::CreatePlaylist { .. }));
        assert!(matches!(removed[1].op, EditOp::AddToPlaylist { .. }));
        assert!(q.failed().unwrap().is_empty());
        // Bekleyene ya da olmayana dokunmaz.
        q.enqueue_at(&EditOp::Save { uri: track(6) }, 1).unwrap();
        let pending = q.pending().unwrap()[0].id;
        assert!(q.dismiss_cascade(pending).unwrap().is_empty());
        assert!(q.dismiss_cascade(9999).unwrap().is_empty());
        assert_eq!(q.pending().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn report_carries_the_applied_ops_in_order_and_rows_show_in_flight() {
        let q = EditQueue::open_in_memory().unwrap();
        let temp = new_temp_id();
        let create = EditOp::CreatePlaylist {
            temp_id: temp.clone(),
            name: "Yeni".into(),
            uris: vec![],
            created: None,
        };
        q.enqueue_at(&create, 0).unwrap();
        q.enqueue_at(
            &EditOp::AddToPlaylist {
                playlist: temp.clone(),
                uris: vec![track(1)],
                position: AddPosition::Last,
            },
            0,
        )
        .unwrap();
        let real = playlist(7);
        let fake = Fake::new(vec![Ok(EditOutcome::Created { uri: real.clone() })]);
        let report = q.process_due(&fake, 0).await.unwrap();
        assert_eq!(report.done.len(), report.done_ops.len());
        assert_eq!(report.done_ops[0], create);
        // Ardıl, oluşturmadan sonra gerçek adresle yazıldı; rapor da onu taşır.
        assert!(
            matches!(&report.done_ops[1], EditOp::AddToPlaylist { playlist, .. } if *playlist == real)
        );
        // Gönderimdeki işlem kuyruk kaydında görünür (sonucu belirsiz).
        q.enqueue_at(&EditOp::Save { uri: track(2) }, 1).unwrap();
        let id = q.pending().unwrap()[0].id;
        assert!(!q.pending().unwrap()[0].in_flight);
        q.set_in_flight(id).unwrap();
        assert!(q.pending().unwrap()[0].in_flight);
    }

    #[test]
    fn same_edit_ignores_only_the_half_created_address() {
        let temp = "local:playlist:1".to_string();
        let mk = |created: Option<String>, name: &str| EditOp::CreatePlaylist {
            temp_id: temp.clone(),
            name: name.into(),
            uris: vec![track(1)],
            created,
        };
        assert!(same_edit(&mk(None, "A"), &mk(Some(playlist(1)), "A")));
        assert!(!same_edit(&mk(None, "A"), &mk(None, "B")));
        assert!(same_edit(
            &EditOp::Save { uri: track(1) },
            &EditOp::Save { uri: track(1) }
        ));
        assert!(!same_edit(
            &EditOp::Save { uri: track(1) },
            &EditOp::Unsave { uri: track(1) }
        ));
    }
}

#[cfg(test)]
mod live;
