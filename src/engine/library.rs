//! Kütüphane: playlist'ler, beğenilen şarkılar ve kaydedilen albümler.
//!
//! Hepsi Spotify'ın iç servislerinden (resmi uygulamanın kullandığı yoldan)
//! alınıyor; Web API kullanılmıyor. Bu sayede Spotify'ın hazırladığı listeler
//! (Discover Weekly, Mix'ler, Blend'ler...) de geliyor. Servisler belgesiz
//! olduğu için Spotify'a giden bütün çağrılar bu dosyada toplanıyor:
//! bir şey kırılırsa düzeltilecek yer burası.
//!
//!   kök liste (rootlist)        → kütüphanedeki playlist'ler
//!   /collection/v2/paging       → beğenilen şarkılar + kaydedilen albümler
//!   playlist / albüm / şarkı    → librespot'un metadata istekleri

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use futures::{StreamExt, stream};
use librespot::{
    core::{Error as SpotifyError, SpotifyUri, error::ErrorKind, session::Session},
    metadata::{Album, Metadata, Playlist, Track, image::ImageSize},
    protocol::{
        extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery},
        extension_kind::ExtensionKind,
        metadata::Track as TrackMessage,
        playlist4_external::SelectedListContent,
    },
};
use protobuf::{CodedInputStream, CodedOutputStream, Message, rt::WireType};

use super::cache::Db;

/// Aynı anda en fazla bu kadar metadata isteği.
const PARALLEL_REQUESTS: usize = 8;
/// Tek bir toplu istekte istenen şarkı sayısı.
const TRACK_BATCH: usize = 100;
/// Spotify "yavaşla" derse (rate limit) en fazla bu kadar tekrar dene.
const MAX_RETRIES: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CollectionId {
    Liked,
    Playlist(String),
    Album(String),
    /// Takip edilen sanatçı (yalnız kütüphane kartı olarak; şarkı listesi
    /// değildir: açılınca sanatçı sayfası gelir, `OpenCollection` gitmez).
    Artist(String),
}

impl CollectionId {
    /// Önbellekteki anahtar: "liked" ya da Spotify URI'si.
    pub fn key(&self) -> &str {
        match self {
            Self::Liked => "liked",
            Self::Playlist(uri) | Self::Album(uri) | Self::Artist(uri) => uri,
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        if key == "liked" {
            Some(Self::Liked)
        } else if key.starts_with("spotify:playlist:") {
            Some(Self::Playlist(key.into()))
        } else if key.starts_with("spotify:album:") {
            Some(Self::Album(key.into()))
        } else if key.starts_with("spotify:artist:") {
            Some(Self::Artist(key.into()))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryItem {
    pub id: CollectionId,
    pub name: String,
    /// "Spotify · 50 şarkı", "Sanatçı" gibi kısa açıklama.
    pub subtitle: String,
    /// Kapak resminin adresi; yoksa boş.
    pub image: String,
}

/// Bir listenin (beğenilenler, çalma listesi, albüm) bir satırı: şarkı ve
/// listeye eklendiği an. Önbellekte `collection_tracks` satırıdır
/// (bkz. `cache.rs`, şema sürümü 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Şarkı adresi (`spotify:track:…`; liste yerel dosya / bölüm de içerebilir).
    pub track: String,
    /// Listeye eklendiği an, unix MİLİSANİYE. Spotify vermediyse (albüm
    /// şarkıları, eklenme anı olmayan öğe) ya da önbellek bu özellikten önce
    /// yazıldıysa `None`.
    pub added_at: Option<i64>,
}

impl Entry {
    /// Eklenme anı bilinmeyen satır.
    pub fn untimed(track: impl Into<String>) -> Self {
        Self {
            track: track.into(),
            added_at: None,
        }
    }
}

/// Spotify'ın öğe zaman damgasından (ms; verilmemişse 0 ya da eksi) satır.
pub(super) fn entry_at(track: String, timestamp_ms: i64) -> Entry {
    Entry {
        track,
        added_at: (timestamp_ms > 0).then_some(timestamp_ms),
    }
}

/// Kenar çubuğundaki bir satırın türü: açılabilen liste (çalma listesi ya da
/// kaydedilmiş albüm; hangisi olduğu adresten anlaşılır, bkz. `CollectionId`)
/// ya da klasör (içinde başka satırlar durur, kendisi açılmaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemKind {
    #[default]
    Playlist,
    Folder,
}

impl ItemKind {
    /// Önbellekteki yazılışı.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Playlist => "playlist",
            Self::Folder => "folder",
        }
    }

    /// Bilinmeyen yazılış (daha yeni bir sürümden) liste sayılır.
    pub fn from_tag(tag: &str) -> Self {
        if tag == "folder" {
            Self::Folder
        } else {
            Self::Playlist
        }
    }
}

/// Kenar çubuğu ağacının bir düğümü: düz `LibraryItem`in klasör bilgisi
/// taşıyan hâli. Sabitleme düğümde değil, ayrı tutulur (`Db::set_pinned`):
/// kütüphane Spotify'dan tazelenirken düğümler baştan yazılır, kullanıcının
/// sabitlemesi bundan etkilenmemeli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryNode {
    /// Liste / albüm için `CollectionId::key()`; klasör için Spotify'ın klasör
    /// işareti (`spotify:start-group:…`).
    pub key: String,
    pub kind: ItemKind,
    /// İçinde durduğu klasörün anahtarı; kökteyse `None`.
    pub parent: Option<String>,
    pub name: String,
    pub subtitle: String,
    pub image: String,
    /// Kütüphaneye eklendiği an, unix milisaniye; bilinmiyorsa `None`
    /// (şema sürümü 6).
    pub added_at: Option<i64>,
    /// Yaratıcının adı ("Yaratıcı" sıralaması için); bilinmiyorsa boş.
    pub creator: String,
}

impl LibraryNode {
    /// Kökteki düz bir öğe.
    pub fn from_item(item: &LibraryItem) -> Self {
        Self {
            key: item.id.key().to_string(),
            kind: ItemKind::Playlist,
            parent: None,
            name: item.name.clone(),
            subtitle: item.subtitle.clone(),
            image: item.image.clone(),
            added_at: None,
            creator: String::new(),
        }
    }

    /// Düz öğe hâli; klasör (ya da tanınmayan adres) için `None`.
    #[allow(dead_code)] // yalnız testlerde kullanılıyor
    pub fn to_item(&self) -> Option<LibraryItem> {
        if self.kind == ItemKind::Folder {
            return None;
        }
        Some(LibraryItem {
            id: CollectionId::from_key(&self.key)?,
            name: self.name.clone(),
            subtitle: self.subtitle.clone(),
            image: self.image.clone(),
        })
    }
}

/// Kapağı olmayan listenin "adresi": ilk şarkıların dört farklı albüm kapağı
/// `|` ile (arayüz 2x2 mozaik çizer). Gerçek adres değildir: yalnız
/// `collection_art` (ve `mosaic_urls`) yorumlar, indirilmeye çalışılmaz.
pub const MOSAIC_PREFIX: &str = "mosaic:";
const MOSAIC_SIDES: usize = 4;

/// Mozaik adresinin dört kapağı; mozaik değilse `None`.
pub fn mosaic_urls(image: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = image
        .strip_prefix(MOSAIC_PREFIX)?
        .split('|')
        .filter(|u| !u.is_empty())
        .collect();
    (parts.len() == MOSAIC_SIDES).then_some(parts)
}

/// Kapağı olmayan listelere önbellekteki şarkılarından kapak bulur: dört
/// farklı albüm kapağı varsa mozaik, 1-3 varsa ilki (tek kapak); şarkıları
/// önbellekte yoksa dokunmaz (arayüz sakin bir yer tutucu çizer). Ağ yok.
pub fn with_mosaics(db: &Db, library: &mut Library) {
    for item in library.playlists.iter_mut().filter(|i| i.image.is_empty()) {
        let Ok(urls) = db.cover_samples(&item.id, MOSAIC_SIDES) else {
            continue;
        };
        item.image = match urls.len() {
            0 => continue,
            n if n >= MOSAIC_SIDES => format!("{MOSAIC_PREFIX}{}", urls.join("|")),
            _ => urls[0].clone(),
        };
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    pub liked_count: usize,
    pub albums: Vec<LibraryItem>,
    pub playlists: Vec<LibraryItem>,
    /// Takip edilen sanatçılar, en son takip edilen önde (Spotify'ın `artist`
    /// koleksiyonu). Ayrı tabloda saklanır (`library_artists`, bkz. `cache.rs`).
    pub artists: Vec<LibraryItem>,
    /// Klasörler, sabitlemeler ve zaman damgaları (kütüphane düzeni).
    pub meta: LibraryMeta,
}

/// Spotify kök listesindeki bir klasör (`start-group` ... `end-group` çifti).
/// Yalnız OKUNUR: klasör oluşturma / taşıma gibi yazma işlemleri Openify'da
/// yok (edit kuyruğunda kök liste işlemi bulunmuyor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Spotify'ın klasör işareti (`spotify:start-group:<kimlik>:<ad>`): hem
    /// kimlik hem `LibraryNode::key`.
    pub key: String,
    pub name: String,
    /// İçinde durduğu klasörün anahtarı; kökteyse `None`.
    pub parent: Option<String>,
    /// Klasör başlarken `playlists`te kaç liste vardı (ondan önce kaç liste
    /// var). Kardeşler arasındaki yeri bundan çıkar: `at`ı küçük olan önce
    /// gelir; eşitlikte klasör önce (işareti listeden önce okundu).
    pub at: usize,
}

/// Kütüphanenin düz listelerinde olmayan, ağaç / sabitleme / sıralama için
/// gereken bilgiler. Anahtar: `CollectionId::key()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryMeta {
    /// Klasörler, kök listedeki sırasıyla (`at` azalmaz).
    pub folders: Vec<Folder>,
    /// Liste anahtarı → içinde durduğu klasörün anahtarı (kökteki yok).
    pub parents: HashMap<String, String>,
    /// Liste / albümün kütüphaneye eklendiği an (unix ms; bilinmeyenler yok).
    pub added: HashMap<String, i64>,
    /// Listenin yaratıcısı (sahibinin kullanıcı adı; Spotify'ınkiler "spotify").
    pub creators: HashMap<String, String>,
    /// Sabitlenenlerin anahtarları (liste, albüm, sanatçı, "liked"),
    /// sabitlenme sırasıyla (en eski önce). Yerel: Spotify'a yazılmaz
    /// (`library_pins`).
    pub pinned: Vec<String>,
}

impl LibraryMeta {
    /// Klasör ağacı var mı?
    pub fn has_folders(&self) -> bool {
        !self.folders.is_empty()
    }
}

/// Klasörün kalıcı kimliği: adı içermeyen kısım (`spotify:start-group:<kimlik>`).
/// Klasör Spotify'da yeniden adlandırılınca anahtar değişir, bu değişmez:
/// açık / kapalı durumu bununla saklanır.
pub fn folder_id(key: &str) -> &str {
    key.match_indices(':')
        .nth(2)
        .map_or(key, |(i, _)| &key[..i])
}

/// Düz kütüphaneden (albümler + listeler) ağaç düğümleri, kenar çubuğu
/// sırasıyla: albümler, sonra listeler; klasörler `at` yerlerinde araya girer.
pub fn library_nodes(library: &Library) -> Vec<LibraryNode> {
    let meta = &library.meta;
    let mut nodes: Vec<LibraryNode> = library
        .albums
        .iter()
        .map(|a| LibraryNode {
            added_at: meta.added.get(a.id.key()).copied(),
            ..LibraryNode::from_item(a)
        })
        .collect();
    let folder_node = |f: &Folder| LibraryNode {
        key: f.key.clone(),
        kind: ItemKind::Folder,
        parent: f.parent.clone(),
        name: f.name.clone(),
        subtitle: String::new(),
        image: String::new(),
        added_at: None,
        creator: String::new(),
    };
    let mut next = 0;
    for (i, p) in library.playlists.iter().enumerate() {
        while next < meta.folders.len() && meta.folders[next].at <= i {
            nodes.push(folder_node(&meta.folders[next]));
            next += 1;
        }
        let key = p.id.key();
        nodes.push(LibraryNode {
            parent: meta.parents.get(key).cloned(),
            added_at: meta.added.get(key).copied(),
            creator: meta.creators.get(key).cloned().unwrap_or_default(),
            ..LibraryNode::from_item(p)
        });
    }
    nodes.extend(meta.folders[next..].iter().map(folder_node));
    nodes
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrackRow {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u32,
    /// Albüm kapağının küçük (64 px) hali; yoksa boş.
    pub image: String,
}

/// Spotify resim kimliğinden adres.
pub fn image_url(id: &[u8]) -> String {
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!("https://i.scdn.co/image/{hex}")
}

/// Albüm kapakları arasından istenen boyuttakini seçer (yoksa ilkini).
pub(super) fn pick_cover(covers: &librespot::metadata::image::Images, size: ImageSize) -> String {
    covers
        .iter()
        .find(|c| c.size == size)
        .or_else(|| covers.first())
        .map(|c| image_url(&c.id.0))
        .unwrap_or_default()
}

/// Spotify'dan tazelenen kütüphane ve içeriği değişmiş (yeniden indirilmesi
/// gereken) listeler.
pub struct Refresh {
    pub library: Library,
    pub changed: Vec<(CollectionId, Vec<u8>)>,
    /// Beğenilenler önbellektekinden farklı mı?
    pub liked_changed: bool,
}

/// Kütüphanenin özetini Spotify'dan alır ve önbelleğe yazar.
///
/// Kenar çubuğu tek bir kök liste isteğinden kurulur (ad, sahip, şarkı sayısı
/// ve sürüm numarası onunla geliyor). Liste içerikleri burada indirilmez;
/// sürümü önbellektekinden farklı olanlar `changed` ile döner.
pub async fn refresh_library(
    session: &Session,
    db: &Db,
    previous: Option<&Library>,
) -> Result<Refresh> {
    // Beğenilenler ve kaydedilen albümler: koleksiyon servisi.
    let collection = collection_items(session, "collection").await?;
    let mut liked = Vec::new();
    let mut album_uris = Vec::new();
    let mut meta = LibraryMeta::default();
    for item in collection {
        if item.uri.starts_with("spotify:track:") {
            liked.push(item);
        } else if item.uri.starts_with("spotify:album:") {
            if item.added_at > 0 {
                meta.added
                    .insert(item.uri.clone(), i64::from(item.added_at) * 1000);
            }
            album_uris.push(item.uri);
        }
    }
    let liked = liked_entries(liked);
    let liked_uris: Vec<String> = liked.iter().map(|e| e.track.clone()).collect();
    // "Değişti" yalnız şarkı sırası için (arayüz listeyi yeniden ister);
    // eklenme anı eksik önbellek (özellikten önce yazılmış) sessizce tamamlanır.
    let liked_changed = db.collection_uris(&CollectionId::Liked)?.as_ref() != Some(&liked_uris);
    if liked_changed || db.collection_entries(&CollectionId::Liked)?.as_ref() != Some(&liked) {
        db.save_collection_entries(&CollectionId::Liked, None, &liked)?;
    }

    // Albümler değişmez: önbellekte olanlar yeniden istenmez.
    let known: HashMap<CollectionId, LibraryItem> = previous
        .map(|l| l.albums.iter().map(|a| (a.id.clone(), a.clone())).collect())
        .unwrap_or_default();
    let known = &known;
    let albums = fetch_all(album_uris, |uri| {
        let id = CollectionId::Album(uri.clone());
        let cached = known.get(&id).cloned();
        let session = session.clone();
        async move {
            if let Some(item) = cached {
                return Ok(item);
            }
            let album = Album::get(&session, &SpotifyUri::from_uri(&uri)?).await?;
            let tracks = album
                .tracks()
                .map(|u| u.to_uri())
                .collect::<Result<Vec<_>, _>>()?;
            db.save_collection(&id, None, &tracks)?;
            Ok(LibraryItem {
                id,
                name: album.name.clone(),
                subtitle: join_artists(album.artists.iter().map(|a| a.name.as_str())),
                image: pick_cover(&album.covers, ImageSize::DEFAULT),
            })
        }
    })
    .await;

    // Takip edilen sanatçılar: koleksiyon servisinin "artist" kümesi. Alınamazsa
    // kütüphanenin geri kalanı yine gelir, sanatçılar önbellekteki hâliyle kalır.
    let known_artists: &[LibraryItem] = previous.map(|l| l.artists.as_slice()).unwrap_or_default();
    let artists = match followed_artists(session, known_artists).await {
        Ok(artists) => artists,
        Err(e) => {
            log::warn!("Takip edilen sanatçılar alınamadı, önbellektekiler kalıyor: {e:#}");
            known_artists.to_vec()
        }
    };

    // Listeler: kök liste.
    //
    // Kök liste her listenin sürüm numarasını veriyor; sürümü değişmeyenler
    // önbellekten aynen alınır. Spotify'ın kendi listelerinde (Mix'ler,
    // Discover Weekly...) kök listedeki ad çevrilmemiş ("2020s Mix") ve şarkı
    // sayısı çoğu zaman 0 geliyor; bunlar sürümü değişince ayrıca istenir.
    let username = session.username();
    let previous_items: HashMap<CollectionId, LibraryItem> = previous
        .map(|l| {
            l.playlists
                .iter()
                .map(|p| (p.id.clone(), p.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut slots: Vec<Option<LibraryItem>> = Vec::new();
    let mut spotify_owned: Vec<(usize, CollectionId, Vec<u8>, String)> = Vec::new();
    let mut changed = Vec::new();
    let Rootlist {
        entries: root_entries,
        folders,
    } = rootlist(session).await?;
    for entry in root_entries {
        if let Some(parent) = &entry.parent {
            meta.parents.insert(entry.uri.clone(), parent.clone());
        }
        if let Some(at) = entry.added_at {
            meta.added.insert(entry.uri.clone(), at);
        }
        let creator = if entry.owner.is_empty() {
            username.as_str()
        } else {
            entry.owner.as_str()
        };
        meta.creators.insert(entry.uri.clone(), creator.to_string());
        let id = CollectionId::Playlist(entry.uri);
        let unchanged = db.revision(&id)?.as_deref() == Some(entry.revision.as_slice());
        if unchanged && let Some(item) = previous_items.get(&id) {
            slots.push(Some(item.clone()));
            continue;
        }
        if entry.owner == "spotify" {
            spotify_owned.push((slots.len(), id, entry.revision, entry.image));
            slots.push(None);
            continue;
        }
        let count = entry.length as usize;
        let subtitle = if entry.owner == username || entry.owner.is_empty() {
            crate::i18n::canon::songs(count)
        } else {
            crate::i18n::canon::followed(count)
        };
        if !unchanged {
            changed.push((id.clone(), entry.revision));
        }
        slots.push(Some(LibraryItem {
            id,
            name: entry.name,
            subtitle,
            image: entry.image,
        }));
    }

    // Spotify'ın listeleri: ad, şarkı sayısı ve şarkı sırası tek istekte.
    let plan: HashMap<String, (usize, Vec<u8>, String)> = spotify_owned
        .iter()
        .map(|(slot, id, rev, image)| (id.key().to_string(), (*slot, rev.clone(), image.clone())))
        .collect();
    let plan = &plan;
    let fetched = fetch_all(
        spotify_owned
            .iter()
            .map(|(_, id, _, _)| id.key().to_string())
            .collect(),
        |uri| {
            let session = session.clone();
            async move {
                let playlist = Playlist::get(&session, &SpotifyUri::from_uri(&uri)?).await?;
                let entries = playlist_entries(&playlist)?;
                let (slot, revision, image) = plan[&uri].clone();
                let id = CollectionId::Playlist(uri);
                db.save_collection_entries(&id, Some(&revision), &entries)?;
                Ok((
                    slot,
                    LibraryItem {
                        id,
                        name: playlist.name().to_string(),
                        subtitle: crate::i18n::canon::spotify(entries.len()),
                        image,
                    },
                ))
            }
        },
    )
    .await;
    for (slot, item) in fetched {
        slots[slot] = Some(item);
    }
    // Alınamayan Spotify listeleri atlanır (bir sonraki tazelemede yeniden denenir).
    // Klasörlerin yeri alınamayan listeler düşünce kayar: `at` yeniden sayılır.
    meta.folders = folders
        .into_iter()
        .map(|f| Folder {
            at: slots[..f.at.min(slots.len())].iter().flatten().count(),
            ..f
        })
        .collect();
    let playlists: Vec<LibraryItem> = slots.into_iter().flatten().collect();
    // Sabitlemeler yerel (`library_pins`): Spotify'dan gelen bir şey değil.
    meta.pinned = db.pinned_keys().unwrap_or_default();

    let library = Library {
        liked_count: liked.len(),
        albums,
        playlists,
        artists,
        meta,
    };
    db.save_library(&library)?;
    Ok(Refresh {
        library,
        changed,
        liked_changed,
    })
}

/// Sürümü değişmiş bir listenin şarkı sırasını indirip önbelleğe yazar.
pub async fn sync_playlist(
    session: &Session,
    db: &Db,
    id: &CollectionId,
    revision: &[u8],
) -> Result<()> {
    let entries = with_retry(|| fetch_collection_entries(session, id)).await?;
    db.save_collection_entries(id, Some(revision), &entries)
}

/// Tek bir toplu istekte adı ve portresi sorulan sanatçı sayısı.
const ARTIST_BATCH: usize = 50;

/// Takip edilen sanatçıların sırası ve önbellekte olmayanları (adı / portresi
/// henüz bilinmeyenler). En son takip edilen önde (`added_at` büyükten küçüğe,
/// eşitlikte Spotify'ın sırası), aynı sanatçı iki kez gelirse bir kez, sanatçı
/// olmayan öğe (başka bir tür) atlanır.
fn plan_followed(items: &[CollectionItem], known: &[LibraryItem]) -> (Vec<String>, Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut sorted: Vec<&CollectionItem> = items
        .iter()
        .filter(|i| i.uri.starts_with("spotify:artist:") && !i.is_removed)
        .filter(|i| seen.insert(i.uri.as_str()))
        .collect();
    sorted.sort_by_key(|i| std::cmp::Reverse(i.added_at));
    let order: Vec<String> = sorted.into_iter().map(|i| i.uri.clone()).collect();
    let have: std::collections::HashSet<&str> = known.iter().map(|k| k.id.key()).collect();
    let missing = order
        .iter()
        .filter(|u| !have.contains(u.as_str()))
        .cloned()
        .collect();
    (order, missing)
}

/// Takip edilen sanatçıları Spotify'dan alır. Adı ve portresi önbellekteki
/// (`known`) kartlardan; yenilerinin ARTIST_V4 ile toplu sorulur. Bilgisi
/// gelmeyen yeni sanatçı bu turda atlanır (sonraki tazelemede yeniden denenir).
async fn followed_artists(session: &Session, known: &[LibraryItem]) -> Result<Vec<LibraryItem>> {
    let items = with_retry(|| collection_items(session, "artist")).await?;
    let (order, missing) = plan_followed(&items, known);
    let mut resolved: HashMap<String, LibraryItem> = known
        .iter()
        .filter(|k| order.iter().any(|u| u == k.id.key()))
        .map(|k| (k.id.key().to_string(), k.clone()))
        .collect();
    for chunk in missing.chunks(ARTIST_BATCH) {
        match super::pages::fetch_artist_cards(session, chunk).await {
            Ok(cards) => {
                for card in cards {
                    resolved.insert(
                        card.uri.clone(),
                        artist_item(&card.uri, &card.name, &card.image),
                    );
                }
            }
            Err(e) => log::warn!("{} sanatçının bilgisi alınamadı: {e:#}", chunk.len()),
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|u| resolved.remove(&u))
        .collect())
}

/// Kütüphanedeki takip edilen sanatçı kartı (altyazı kanonik "Sanatçı").
pub fn artist_item(uri: &str, name: &str, image: &str) -> LibraryItem {
    LibraryItem {
        id: CollectionId::Artist(uri.to_string()),
        name: name.to_string(),
        subtitle: crate::i18n::canon::ARTIST.to_string(),
        image: image.to_string(),
    }
}

/// Beğenilenler: şarkılar, en son eklenen başta. Spotify'ın `added_at`'i
/// saniyedir; önbellekte (diğer kaynaklarla aynı) milisaniye tutulur.
fn liked_entries(items: Vec<CollectionItem>) -> Vec<Entry> {
    let mut items: Vec<CollectionItem> = items
        .into_iter()
        .filter(|i| i.uri.starts_with("spotify:track:"))
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.added_at));
    items
        .into_iter()
        .map(|i| entry_at(i.uri, i64::from(i.added_at) * 1000))
        .collect()
}

/// Listenin satırları; her öğenin eklenme anı Spotify'ın öğe özniteliğinden
/// (`timestamp`, ms; verilmemişse `None`).
fn playlist_entries(playlist: &Playlist) -> Result<Vec<Entry>> {
    playlist
        .contents
        .items
        .iter()
        .map(|item| {
            Ok(entry_at(
                item.id.to_uri()?,
                item.attributes.timestamp.as_timestamp_ms(),
            ))
        })
        .collect()
}

async fn fetch_collection_entries(session: &Session, id: &CollectionId) -> Result<Vec<Entry>> {
    let entries = match id {
        CollectionId::Liked => liked_entries(collection_items(session, "collection").await?),
        CollectionId::Playlist(uri) => playlist_entries(
            &Playlist::get(session, &SpotifyUri::from_uri(uri)?)
                .await
                .context("Liste alınamadı")?,
        )?,
        // Albümün şarkılarının "eklenme anı" yok.
        CollectionId::Album(uri) => Album::get(session, &SpotifyUri::from_uri(uri)?)
            .await
            .context("Albüm alınamadı")?
            .tracks()
            .map(|u| Ok(Entry::untimed(u.to_uri()?)))
            .collect::<Result<_>>()?,
        // Takip edilen sanatçının şarkı listesi yok (sayfası açılır, `OpenCollection` gitmez).
        CollectionId::Artist(_) => Vec::new(),
    };
    Ok(entries)
}

fn only_tracks(uris: Vec<String>) -> Vec<String> {
    // Podcast bölümleri ve yerel dosyalar şimdilik atlanıyor.
    uris.into_iter()
        .filter(|u| u.starts_with("spotify:track:"))
        .collect()
}

/// Önbellekteki haliyle bir listenin şarkıları ve hepsinin bilgisinin
/// önbellekte olup olmadığı. Liste hiç kaydedilmediyse `None`.
pub fn cached_tracks(db: &Db, id: &CollectionId) -> Result<Option<(Arc<Vec<TrackRow>>, bool)>> {
    let Some(uris) = db.collection_uris(id)? else {
        return Ok(None);
    };
    let uris = only_tracks(uris);
    // Aynı şarkı listede birden çok kez olabilir: her sıra kendi satırını alır
    // (`remove` ikincisini düşürüyordu, yinelenenler listeden kayboluyordu).
    let found = db.tracks(&uris)?;
    let complete = uris.iter().all(|u| found.contains_key(u));
    let rows = uris.iter().filter_map(|u| found.get(u).cloned()).collect();
    Ok(Some((Arc::new(rows), complete)))
}

/// Bir listenin şarkıları: önce önbellek, eksikler Spotify'dan.
pub async fn load_tracks(
    session: &Session,
    db: &Db,
    id: &CollectionId,
) -> Result<Arc<Vec<TrackRow>>> {
    let uris = match db.collection_uris(id)? {
        Some(uris) => uris,
        None => {
            let entries = with_retry(|| fetch_collection_entries(session, id)).await?;
            db.save_collection_entries(id, None, &entries)?;
            entries.into_iter().map(|e| e.track).collect()
        }
    };
    let uris = only_tracks(uris);

    let mut found = db.tracks(&uris)?;
    let missing: Vec<String> = uris
        .iter()
        .filter(|u| !found.contains_key(*u))
        .cloned()
        .collect();
    if !missing.is_empty() {
        let fetched = fetch_tracks(session, missing).await;
        db.save_tracks(&fetched)?;
        found.extend(fetched.into_iter().map(|r| (r.uri.clone(), r)));
    }
    // Yinelenen şarkılar da kendi sırasında kalır (bkz. `cached_tracks`).
    Ok(Arc::new(
        uris.iter().filter_map(|u| found.get(u).cloned()).collect(),
    ))
}

/// Listelerde geçen ama bilgisi henüz indirilmemiş bütün şarkıları arka
/// planda indirir (Adım 7'deki arama da bunlardan yararlanacak).
pub async fn fill_missing_tracks(session: &Session, db: &Db) -> Result<usize> {
    let mut total = 0;
    loop {
        let missing = db.missing_tracks(TRACK_BATCH * 5)?;
        if missing.is_empty() {
            return Ok(total);
        }
        let asked = missing.len();
        let fetched = fetch_tracks(session, missing).await;
        if fetched.is_empty() {
            // Spotify bu şarkıları vermiyor; sonsuz döngüye girme.
            log::warn!("{asked} şarkının bilgisi alınamadı, sonraki açılışta yeniden denenecek");
            return Ok(total);
        }
        total += fetched.len();
        db.save_tracks(&fetched)?;
    }
}

/// Şarkı bilgilerini 100'lük gruplar halinde, tek istekte alır (resmi
/// uygulamanın yaptığı gibi). Tek tek istemek Spotify'ın sınırına takılıyor.
/// Sıra korunur; bilgisi gelmeyen şarkılar atlanır.
pub(super) async fn fetch_tracks(session: &Session, uris: Vec<String>) -> Vec<TrackRow> {
    let batches: Vec<Vec<String>> = uris.chunks(TRACK_BATCH).map(<[String]>::to_vec).collect();
    let mut found: HashMap<String, TrackRow> = HashMap::new();
    for batch in batches {
        match with_retry(|| fetch_track_batch(session, &batch)).await {
            Ok(rows) => found.extend(rows.into_iter().map(|r| (r.uri.clone(), r))),
            Err(e) => log::warn!("{} şarkılık grup alınamadı: {e:#}", batch.len()),
        }
    }
    let missing = uris.iter().filter(|u| !found.contains_key(*u)).count();
    if missing > 0 {
        log::warn!("{missing} şarkının bilgisi gelmedi");
    }
    uris.into_iter().filter_map(|u| found.remove(&u)).collect()
}

async fn fetch_track_batch(session: &Session, uris: &[String]) -> Result<Vec<TrackRow>> {
    let request = BatchedEntityRequest {
        entity_request: uris
            .iter()
            .map(|uri| EntityRequest {
                entity_uri: uri.clone(),
                query: vec![ExtensionQuery {
                    extension_kind: ExtensionKind::TRACK_V4.into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let response = session.spclient().get_extended_metadata(request).await?;

    let mut rows = Vec::new();
    for array in &response.extended_metadata {
        for data in &array.extension_data {
            let Some(any) = data.extension_data.as_ref() else {
                continue;
            };
            let Ok(uri) = SpotifyUri::from_uri(&data.entity_uri) else {
                continue;
            };
            let Ok(message) = TrackMessage::parse_from_bytes(&any.value) else {
                continue;
            };
            let Ok(track) = Track::parse(&message, &uri) else {
                continue;
            };
            super::traits::note(
                &data.entity_uri,
                super::traits::from_spotify(track.is_explicit, track.popularity),
            );
            rows.push(TrackRow {
                uri: data.entity_uri.clone(),
                name: track.name.clone(),
                artists: join_artists(track.artists.iter().map(|a| a.name.as_str())),
                album: track.album.name.clone(),
                duration_ms: track.duration.max(0) as u32,
                image: pick_cover(&track.album.covers, ImageSize::SMALL),
            });
        }
    }
    Ok(rows)
}

/// Spotify "yavaşla" derse 0,5 / 1 / 2 / 4 sn bekleyip tekrar dener.
pub(super) async fn with_retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut delay = Duration::from_millis(500);
    for attempt in 0..=MAX_RETRIES {
        match f().await {
            Err(e) if attempt < MAX_RETRIES && is_rate_limited(&e) => {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            other => return other,
        }
    }
    unreachable!()
}

fn is_rate_limited(e: &anyhow::Error) -> bool {
    e.downcast_ref::<SpotifyError>()
        .is_some_and(|e| e.kind == ErrorKind::ResourceExhausted)
}

/// Her öğe için `fetch`'i aynı anda en fazla `PARALLEL_REQUESTS` istekle
/// çalıştırır; sırayı korur, alınamayanları atlar.
async fn fetch_all<T, F, Fut>(uris: Vec<String>, fetch: F) -> Vec<T>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let fetch = &fetch;
    stream::iter(uris.into_iter().map(|uri| async move {
        with_retry(|| fetch(uri.clone()))
            .await
            .map_err(|e| log::warn!("{uri} alınamadı: {e:#}"))
            .ok()
    }))
    .buffered(PARALLEL_REQUESTS)
    .filter_map(|x| async move { x })
    .collect()
    .await
}

pub(super) fn join_artists<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names.collect::<Vec<_>>().join(", ")
}

// ── Kök liste ───────────────────────────────────────────────────────────

struct RootEntry {
    uri: String,
    name: String,
    owner: String,
    length: i32,
    revision: Vec<u8>,
    image: String,
    /// Kütüphaneye eklendiği an (ms); Spotify vermediyse `None`.
    added_at: Option<i64>,
    /// İçinde durduğu klasörün anahtarı; kökteyse `None`.
    parent: Option<String>,
}

/// Kök listenin bir satırı, ağaç kurmak için: klasör başlıyor / bitiyor ya da
/// bir liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tok {
    /// `spotify:start-group:<kimlik>:<ad>`: (tam işaret, kimlik, çözülmüş ad).
    Start {
        key: String,
        id: String,
        name: String,
    },
    /// `spotify:end-group:<kimlik>`.
    End {
        id: String,
    },
    Playlist,
}

/// Klasör işaretini çözer: `spotify:start-group:<kimlik>:<ad>` (ad yüzde
/// kodlu olabilir) ya da `spotify:end-group:<kimlik>`. İşaret değilse `None`.
pub(super) fn parse_marker(uri: &str) -> Option<Tok> {
    let rest = uri.strip_prefix("spotify:")?;
    if let Some(rest) = rest.strip_prefix("start-group:") {
        let (id, name) = rest.split_once(':').unwrap_or((rest, ""));
        return Some(Tok::Start {
            key: uri.to_string(),
            id: id.to_string(),
            name: percent_decode(name),
        });
    }
    rest.strip_prefix("end-group:")
        .map(|id| Tok::End { id: id.to_string() })
}

/// `%XX` dizilerini ve `+`'ları çözer (Spotify klasör adını böyle yazıyor).
/// Geçersiz dizi olduğu gibi kalır; bozuk UTF-8 yerine karakteri koyar.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if bytes
                .get(i + 1..i + 3)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) =>
            {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'%'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Kök listenin sırasından klasör ağacı: klasörler (`at` = kendisinden önceki
/// liste sayısı) ve her listenin (sırayla) içinde durduğu klasörün anahtarı.
/// Hoşgörülü: kapanmayan klasör sona kadar sürer; eşleşmeyen kapanış yok sayılır;
/// iç içe klasörler kapanışta kimliğe göre kapanır.
pub(super) fn fold_tree(tokens: &[Tok]) -> (Vec<Folder>, Vec<Option<String>>) {
    let mut folders = Vec::new();
    let mut parents = Vec::new();
    let mut open: Vec<(String, String)> = Vec::new(); // (kimlik, anahtar)
    let mut lists = 0;
    for tok in tokens {
        match tok {
            Tok::Start { key, id, name } => {
                folders.push(Folder {
                    key: key.clone(),
                    name: name.clone(),
                    parent: open.last().map(|(_, k)| k.clone()),
                    at: lists,
                });
                open.push((id.clone(), key.clone()));
            }
            Tok::End { id } => {
                if let Some(depth) = open.iter().rposition(|(i, _)| i == id) {
                    open.truncate(depth);
                }
            }
            Tok::Playlist => {
                parents.push(open.last().map(|(_, k)| k.clone()));
                lists += 1;
            }
        }
    }
    (folders, parents)
}

/// Kök listeden gelen bütün bilgi: listeler ve klasör ağacı.
struct Rootlist {
    entries: Vec<RootEntry>,
    /// Klasörler (`at` = kökten gelen sıradaki yeri; çıkarılamayan listeler
    /// sonradan düşerse `refresh_library` yeniden hesaplar).
    folders: Vec<Folder>,
}

/// Kütüphanedeki playlist'ler ve klasörleri, Spotify'daki sırasıyla.
/// Spotify'ın erişim vermediği listeler (403 vb.) atlanır.
async fn rootlist(session: &Session) -> Result<Rootlist> {
    let bytes = with_retry(|| async { Ok(session.spclient().get_rootlist(0, Some(1000)).await?) })
        .await
        .context("Kök liste alınamadı")?;
    let list = SelectedListContent::parse_from_bytes(&bytes)?;
    let contents = &list.contents;
    let mut entries = Vec::new();
    let mut tokens = Vec::new();
    for (i, item) in contents.items.iter().enumerate() {
        let uri = item.uri();
        if let Some(marker) = parse_marker(uri) {
            tokens.push(marker);
            continue;
        }
        if !uri.starts_with("spotify:playlist:") {
            continue;
        }
        let Some(meta) = contents.meta_items.get(i) else {
            continue;
        };
        let status = meta.status_code();
        if status != 0 && status != 200 {
            log::warn!("{uri} atlandı (Spotify durum kodu {status})");
            continue;
        }
        tokens.push(Tok::Playlist);
        entries.push(RootEntry {
            uri: uri.to_string(),
            name: meta.attributes.name().to_string(),
            owner: meta.owner_username().to_string(),
            length: meta.length(),
            revision: meta.revision().to_vec(),
            image: list_image(&meta.attributes),
            added_at: (item.attributes.timestamp() > 0).then(|| item.attributes.timestamp()),
            parent: None,
        });
    }
    let (folders, parents) = fold_tree(&tokens);
    for (entry, parent) in entries.iter_mut().zip(parents) {
        entry.parent = parent;
    }
    Ok(Rootlist { entries, folders })
}

/// Canlı denetim için (yalnız sınamalar): kök listenin ham sırası, klasör
/// işaretleri dahil, Spotify'ın verdiği gibi.
#[cfg(test)]
pub(crate) async fn raw_rootlist_uris(session: &Session) -> Result<Vec<String>> {
    let bytes = session
        .spclient()
        .get_rootlist(0, Some(1000))
        .await
        .context("Kök liste alınamadı")?;
    let list = SelectedListContent::parse_from_bytes(&bytes)?;
    Ok(list
        .contents
        .items
        .iter()
        .map(|i| i.uri().to_string())
        .collect())
}

/// Canlı denetim için (yalnız sınamalar): adresleri gerçek şarkılara çözer.
#[cfg(test)]
pub(crate) async fn resolve_tracks(session: &Session, uris: &[String]) -> Result<Vec<TrackRow>> {
    fetch_track_batch(session, uris).await
}

/// Canlı denetim için (yalnız sınamalar): "collection" kümesindeki bütün
/// adresler (şarkılar ve albümler), silinmişler hariç.
#[cfg(test)]
pub(crate) async fn raw_collection_uris(session: &Session) -> Result<Vec<String>> {
    Ok(collection_items(session, "collection")
        .await?
        .into_iter()
        .map(|i| i.uri)
        .collect())
}

/// Canlı denetim için (yalnız sınamalar): "artist" kümesindeki bütün adresler
/// (takip edilen sanatçılar), silinmişler hariç.
#[cfg(test)]
pub(crate) async fn raw_artist_uris(session: &Session) -> Result<Vec<String>> {
    Ok(collection_items(session, "artist")
        .await?
        .into_iter()
        .map(|i| i.uri)
        .collect())
}

/// Listenin kapağı: kendi resmi varsa o, yoksa Spotify'ın verdiği boyutlu
/// adreslerden "default" (ya da ilki). İkisi de yoksa boş: arayüz yer
/// tutucu gösterir.
fn list_image(attributes: &librespot::protocol::playlist4_external::ListAttributes) -> String {
    if !attributes.picture().is_empty() {
        return image_url(attributes.picture());
    }
    attributes
        .picture_size
        .iter()
        .find(|p| p.target_name() == "default")
        .or_else(|| attributes.picture_size.first())
        .map(|p| p.url().to_string())
        .unwrap_or_default()
}

// ── Koleksiyon servisi ──────────────────────────────────────────────────
//
// collection2v2.proto librespot'ta derlenmiyor; iki küçük mesaj elle
// kodlanıyor. Servis, `application/x-protobuf` değil kendi içerik türünü
// bekliyor; yanlış türde 400 dönüyor.

const COLLECTION_CONTENT_TYPE: &str = "application/vnd.collection-v2.spotify.proto";
const COLLECTION_PAGE_SIZE: i32 = 300;
/// Sonsuz döngüye karşı üst sınır (300 × 200 = 60.000 öğe).
const COLLECTION_MAX_PAGES: usize = 200;

#[derive(Debug, Clone, PartialEq)]
struct CollectionItem {
    uri: String,
    added_at: i32,
    is_removed: bool,
}

async fn collection_items(session: &Session, set: &str) -> Result<Vec<CollectionItem>> {
    let username = session.username();
    let mut items = Vec::new();
    let mut token = String::new();

    for _ in 0..COLLECTION_MAX_PAGES {
        let body = encode_page_request(&username, set, &token, COLLECTION_PAGE_SIZE)?;
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::CONTENT_TYPE, COLLECTION_CONTENT_TYPE.parse()?);
        headers.insert(http::header::ACCEPT, COLLECTION_CONTENT_TYPE.parse()?);
        let bytes = session
            .spclient()
            .request(
                &http::Method::POST,
                "/collection/v2/paging",
                Some(headers),
                Some(&body),
            )
            .await
            .with_context(|| format!("Koleksiyon \"{set}\" alınamadı"))?;

        let (page, next) = decode_page_response(&bytes)?;
        items.extend(page.into_iter().filter(|i| !i.is_removed));
        if next.is_empty() {
            break;
        }
        token = next;
    }
    Ok(items)
}

/// PageRequest { username = 1; set = 2; pagination_token = 3; limit = 4; }
fn encode_page_request(username: &str, set: &str, token: &str, limit: i32) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut os = CodedOutputStream::vec(&mut body);
    os.write_string(1, username)?;
    os.write_string(2, set)?;
    if !token.is_empty() {
        os.write_string(3, token)?;
    }
    os.write_int32(4, limit)?;
    os.flush()?;
    drop(os);
    Ok(body)
}

/// PageResponse { repeated CollectionItem items = 1; next_page_token = 2; }
/// CollectionItem { uri = 1; added_at = 2; is_removed = 3; }
fn decode_page_response(bytes: &[u8]) -> Result<(Vec<CollectionItem>, String)> {
    let mut items = Vec::new();
    let mut next = String::new();
    let mut is = CodedInputStream::from_bytes(bytes);
    while let Some(tag) = is.read_raw_tag_or_eof()? {
        match tag >> 3 {
            1 => {
                let raw = is.read_bytes()?;
                let mut it = CodedInputStream::from_bytes(&raw);
                let mut item = CollectionItem {
                    uri: String::new(),
                    added_at: 0,
                    is_removed: false,
                };
                while let Some(t) = it.read_raw_tag_or_eof()? {
                    match t >> 3 {
                        1 => item.uri = it.read_string()?,
                        2 => item.added_at = it.read_int32()?,
                        3 => item.is_removed = it.read_bool()?,
                        _ => it.skip_field(wire_type(t)?)?,
                    }
                }
                items.push(item);
            }
            2 => next = is.read_string()?,
            _ => is.skip_field(wire_type(tag)?)?,
        }
    }
    Ok((items, next))
}

fn wire_type(tag: u32) -> Result<WireType> {
    WireType::new(tag & 7).context("Geçersiz protobuf alanı")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aynı şarkı listede birden çok kez durur: önbellekten okuma hepsini döner
    /// (sıralar Spotify'daki sıralarla aynı kalır) ve liste "eksiksiz" sayılır.
    #[test]
    fn cached_tracks_keep_repeated_songs_in_place() {
        let db = Db::open_in_memory().unwrap();
        let row = |n: u32| TrackRow {
            uri: format!("spotify:track:{n}"),
            name: format!("S{n}"),
            artists: "A".into(),
            album: "B".into(),
            duration_ms: 1,
            image: String::new(),
        };
        db.save_tracks(&[row(1), row(2)]).unwrap();
        let id = CollectionId::Playlist("spotify:playlist:1".into());
        let entries: Vec<Entry> = [1, 2, 1, 1, 2]
            .iter()
            .map(|n| Entry::untimed(row(*n).uri))
            .collect();
        db.save_collection_entries(&id, Some(b"r"), &entries)
            .unwrap();
        let (rows, complete) = cached_tracks(&db, &id).unwrap().unwrap();
        assert!(
            complete,
            "iki farklı şarkının ikisi de bilindiği için eksiksiz"
        );
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["S1", "S2", "S1", "S1", "S2"]
        );
        // Bilgisi eksik şarkı yine "eksiksiz değil".
        let mut more = entries.clone();
        more.push(Entry::untimed("spotify:track:3"));
        db.save_collection_entries(&id, Some(b"r"), &more).unwrap();
        let (rows, complete) = cached_tracks(&db, &id).unwrap().unwrap();
        assert!(!complete);
        assert_eq!(rows.len(), 5);
    }

    fn encode_item(os: &mut CodedOutputStream, uri: &str, added_at: i32, removed: bool) {
        let mut inner = Vec::new();
        {
            let mut is = CodedOutputStream::vec(&mut inner);
            is.write_string(1, uri).unwrap();
            is.write_int32(2, added_at).unwrap();
            is.write_bool(3, removed).unwrap();
            // Bilinmeyen bir alan atlanmalı.
            is.write_string(9, "x").unwrap();
            is.flush().unwrap();
        }
        os.write_bytes(1, &inner).unwrap();
    }

    #[test]
    fn decodes_collection_page() {
        let mut bytes = Vec::new();
        {
            let mut os = CodedOutputStream::vec(&mut bytes);
            encode_item(&mut os, "spotify:track:a", 10, false);
            encode_item(&mut os, "spotify:album:b", 20, true);
            os.write_string(2, "sonraki").unwrap();
            os.flush().unwrap();
        }
        let (items, next) = decode_page_response(&bytes).unwrap();
        assert_eq!(next, "sonraki");
        assert_eq!(
            items,
            vec![
                CollectionItem {
                    uri: "spotify:track:a".into(),
                    added_at: 10,
                    is_removed: false
                },
                CollectionItem {
                    uri: "spotify:album:b".into(),
                    added_at: 20,
                    is_removed: true
                },
            ]
        );
    }

    #[test]
    fn page_request_roundtrip() {
        let body = encode_page_request("kullanici", "collection", "tok", 300).unwrap();
        let mut is = CodedInputStream::from_bytes(&body);
        let mut fields = Vec::new();
        while let Some(tag) = is.read_raw_tag_or_eof().unwrap() {
            match tag >> 3 {
                4 => fields.push(is.read_int32().unwrap().to_string()),
                _ => fields.push(is.read_string().unwrap()),
            }
        }
        assert_eq!(fields, ["kullanici", "collection", "tok", "300"]);
    }

    fn collected(uri: &str, added_at: i32) -> CollectionItem {
        CollectionItem {
            uri: uri.into(),
            added_at,
            is_removed: false,
        }
    }

    #[test]
    fn liked_entries_are_newest_first_in_milliseconds_and_tracks_only() {
        let entries = liked_entries(vec![
            collected("spotify:track:old", 100),
            collected("spotify:album:x", 500),
            collected("spotify:track:new", 300),
            collected("spotify:track:nodate", 0),
        ]);
        assert_eq!(
            entries,
            vec![
                Entry {
                    track: "spotify:track:new".into(),
                    added_at: Some(300_000)
                },
                Entry {
                    track: "spotify:track:old".into(),
                    added_at: Some(100_000)
                },
                Entry {
                    track: "spotify:track:nodate".into(),
                    added_at: None
                },
            ]
        );
    }

    #[test]
    fn followed_artists_are_newest_first_deduplicated_and_artist_only() {
        let known = vec![artist_item("spotify:artist:b", "B", "img-b")];
        let (order, missing) = plan_followed(
            &[
                collected("spotify:artist:a", 100),
                collected("spotify:artist:b", 300),
                collected("spotify:album:x", 999),
                collected("spotify:artist:a", 50),
                collected("spotify:artist:c", 200),
                CollectionItem {
                    uri: "spotify:artist:gone".into(),
                    added_at: 400,
                    is_removed: true,
                },
            ],
            &known,
        );
        // En son takip edilen önde; tekrar ve albüm yok; silinen yok.
        assert_eq!(
            order,
            ["spotify:artist:b", "spotify:artist:c", "spotify:artist:a"]
        );
        // Yalnız önbellekte olmayanlar Spotify'dan sorulur (b bilinen).
        assert_eq!(missing, ["spotify:artist:c", "spotify:artist:a"]);
        // Takip yoksa hiçbir şey sorulmaz.
        assert_eq!(plan_followed(&[], &known), (vec![], vec![]));
    }

    #[test]
    fn artist_cards_use_the_canonical_subtitle_and_roundtrip_their_key() {
        let item = artist_item("spotify:artist:a", "Tarkan", "img");
        assert_eq!(item.id, CollectionId::Artist("spotify:artist:a".into()));
        assert_eq!(item.id.key(), "spotify:artist:a");
        assert_eq!(
            CollectionId::from_key("spotify:artist:a"),
            Some(item.id.clone())
        );
        assert_eq!(item.subtitle, crate::i18n::canon::ARTIST);
        // Önbellekten dönen anahtar başka türe karışmaz.
        assert_eq!(
            CollectionId::from_key("spotify:playlist:a"),
            Some(CollectionId::Playlist("spotify:playlist:a".into()))
        );
        assert_eq!(CollectionId::from_key("spotify:show:a"), None);
    }

    #[test]
    fn missing_timestamps_stay_unknown() {
        assert_eq!(entry_at("spotify:track:a".into(), 0).added_at, None);
        assert_eq!(entry_at("spotify:track:a".into(), -5).added_at, None);
        assert_eq!(
            entry_at("spotify:track:a".into(), 1_700_000_000_000).added_at,
            Some(1_700_000_000_000)
        );
        assert_eq!(Entry::untimed("spotify:track:a").added_at, None);
    }

    #[test]
    fn nodes_roundtrip_with_items_and_folders_are_not_items() {
        let item = LibraryItem {
            id: CollectionId::Playlist("spotify:playlist:a".into()),
            name: "A".into(),
            subtitle: "3 şarkı".into(),
            image: "img".into(),
        };
        let node = LibraryNode::from_item(&item);
        assert_eq!(
            (node.kind, node.parent.as_deref()),
            (ItemKind::Playlist, None)
        );
        assert_eq!(node.to_item().as_ref(), Some(&item));
        let folder = LibraryNode {
            key: "spotify:start-group:f1:Klasör".into(),
            kind: ItemKind::Folder,
            parent: None,
            name: "Klasör".into(),
            subtitle: String::new(),
            image: String::new(),
            added_at: None,
            creator: String::new(),
        };
        assert_eq!(folder.to_item(), None);
        assert_eq!(ItemKind::from_tag(ItemKind::Folder.tag()), ItemKind::Folder);
        assert_eq!(ItemKind::from_tag("playlist"), ItemKind::Playlist);
        assert_eq!(ItemKind::from_tag("bilinmeyen"), ItemKind::Playlist);
    }

    fn start(id: &str, name: &str) -> Tok {
        Tok::Start {
            key: format!("spotify:start-group:{id}:{name}"),
            id: id.into(),
            name: name.into(),
        }
    }

    fn end(id: &str) -> Tok {
        Tok::End { id: id.into() }
    }

    #[test]
    fn folder_markers_are_parsed_with_encoded_names() {
        assert_eq!(
            parse_marker("spotify:start-group:8c3a1f:Yaz+Listeleri"),
            Some(Tok::Start {
                key: "spotify:start-group:8c3a1f:Yaz+Listeleri".into(),
                id: "8c3a1f".into(),
                name: "Yaz Listeleri".into()
            })
        );
        assert!(
            matches!(parse_marker("spotify:start-group:a:Gece%20Odası%21"), Some(Tok::Start { name, .. }) if name == "Gece Odası!")
        );
        assert!(
            matches!(parse_marker("spotify:start-group:a:%C3%87ay"), Some(Tok::Start { name, .. }) if name == "Çay")
        );
        // Adı iki noktalı klasör: ad kalan her şey. Adsız klasör: boş ad.
        assert!(
            matches!(parse_marker("spotify:start-group:a:Rock: 90lar"), Some(Tok::Start { name, .. }) if name == "Rock: 90lar")
        );
        assert!(
            matches!(parse_marker("spotify:start-group:a"), Some(Tok::Start { name, .. }) if name.is_empty())
        );
        // Bozuk yüzde dizisi olduğu gibi kalır (çökmez).
        assert!(
            matches!(parse_marker("spotify:start-group:a:100%"), Some(Tok::Start { name, .. }) if name == "100%")
        );
        assert!(
            matches!(parse_marker("spotify:start-group:a:%zz"), Some(Tok::Start { name, .. }) if name == "%zz")
        );
        assert_eq!(
            parse_marker("spotify:end-group:8c3a1f"),
            Some(Tok::End {
                id: "8c3a1f".into()
            })
        );
        assert_eq!(parse_marker("spotify:playlist:x"), None);
        assert_eq!(parse_marker("spotify:track:x"), None);
    }

    #[test]
    fn root_order_becomes_a_nested_folder_tree() {
        // A, [F1: B, [F2: C], D], E, [F3 boş]
        let tokens = [
            Tok::Playlist,
            start("1", "F1"),
            Tok::Playlist,
            start("2", "F2"),
            Tok::Playlist,
            end("2"),
            Tok::Playlist,
            end("1"),
            Tok::Playlist,
            start("3", "F3"),
            end("3"),
        ];
        let (folders, parents) = fold_tree(&tokens);
        let f1 = "spotify:start-group:1:F1";
        let f2 = "spotify:start-group:2:F2";
        assert_eq!(
            parents,
            [
                None,
                Some(f1.into()),
                Some(f2.into()),
                Some(f1.into()),
                None
            ]
        );
        let shape: Vec<(&str, Option<&str>, usize)> = folders
            .iter()
            .map(|f| (f.name.as_str(), f.parent.as_deref(), f.at))
            .collect();
        assert_eq!(
            shape,
            [("F1", None, 1), ("F2", Some(f1), 2), ("F3", None, 5)]
        );
        assert!(
            folders.windows(2).all(|w| w[0].at <= w[1].at),
            "`at` azalmaz"
        );
    }

    #[test]
    fn broken_group_markers_do_not_lose_lists() {
        // Kapanmayan klasör sona kadar sürer; eşleşmeyen kapanış yok sayılır.
        let (folders, parents) = fold_tree(&[
            Tok::Playlist,
            start("1", "A"),
            Tok::Playlist,
            end("zz"),
            Tok::Playlist,
        ]);
        assert_eq!(folders.len(), 1);
        assert_eq!(
            parents,
            [
                None,
                Some("spotify:start-group:1:A".into()),
                Some("spotify:start-group:1:A".into())
            ]
        );
        // Dıştakini kapatan işaret içtekini de kapatır.
        let (_, parents) = fold_tree(&[start("1", "A"), start("2", "B"), end("1"), Tok::Playlist]);
        assert_eq!(parents, [None]);
        let (folders, parents) = fold_tree(&[]);
        assert!(folders.is_empty() && parents.is_empty());
    }

    #[test]
    fn library_nodes_put_folders_back_between_their_lists() {
        let list = |n: u32| LibraryItem {
            id: CollectionId::Playlist(format!("spotify:playlist:{n}")),
            name: format!("L{n}"),
            subtitle: "3 şarkı".into(),
            image: String::new(),
        };
        let (folders, parents) = fold_tree(&[
            Tok::Playlist,
            start("1", "F"),
            Tok::Playlist,
            Tok::Playlist,
            end("1"),
            Tok::Playlist,
        ]);
        let mut library = Library {
            albums: vec![LibraryItem {
                id: CollectionId::Album("spotify:album:a".into()),
                name: "Albüm".into(),
                subtitle: "X".into(),
                image: String::new(),
            }],
            playlists: (1..=4).map(list).collect(),
            ..Default::default()
        };
        for (i, p) in parents.iter().enumerate() {
            if let Some(p) = p {
                library
                    .meta
                    .parents
                    .insert(format!("spotify:playlist:{}", i + 1), p.clone());
            }
        }
        library.meta.folders = folders;
        library.meta.added.insert("spotify:album:a".into(), 5_000);
        library
            .meta
            .added
            .insert("spotify:playlist:3".into(), 9_000);
        library
            .meta
            .creators
            .insert("spotify:playlist:3".into(), "ayse".into());
        let nodes = library_nodes(&library);
        let seq: Vec<(&str, &str)> = nodes
            .iter()
            .map(|n| (n.name.as_str(), n.parent.as_deref().unwrap_or("-")))
            .collect();
        assert_eq!(
            seq,
            [
                ("Albüm", "-"),
                ("L1", "-"),
                ("F", "-"),
                ("L2", "spotify:start-group:1:F"),
                ("L3", "spotify:start-group:1:F"),
                ("L4", "-")
            ]
        );
        assert_eq!(nodes[0].added_at, Some(5_000));
        assert_eq!(
            (nodes[4].added_at, nodes[4].creator.as_str()),
            (Some(9_000), "ayse")
        );
        assert_eq!(nodes[2].kind, ItemKind::Folder);
        // Klasörsüz kütüphane eskisi gibi düz yazılır.
        library.meta = LibraryMeta::default();
        assert!(
            library_nodes(&library)
                .iter()
                .all(|n| n.kind == ItemKind::Playlist && n.parent.is_none())
        );
    }
}
