//! Kütüphane araçları (imza özelliği): yinelenen bulucu, birleştir / böl,
//! kalıcı sıralama, yedek dışa aktarımı ve yerel dinleme istatistiği.
//!
//! Burası motor tarafı: saf kararlar (hangi satırlar yinelenen, hangi sırayla
//! taşınacak, hangi gruplara bölünecek, bir çalma ne zaman sayılır, istatistik
//! nasıl toplanır) ve yerel veri (çalma geçmişi tablosu, yedek dosyası). Arayüz
//! `ui/tools.rs`'te.
//!
//! İki kural her yerde geçerli:
//!   - Spotify'a yazan her işlem MEVCUT edit kuyruğundan geçer (`EditOp`:
//!     şarkı çıkar, liste oluştur, öğe taşı). Bu modül yeni bir canlı yazma
//!     türü eklemez; yalnız hangi `EditOp`'ların kurulacağını hesaplar.
//!   - Çalma geçmişi yalnız yerelde durur (library.db `play_history`), Spotify'a
//!     gitmez ve gizli oturumda yazılmaz (`stats::private_session`).

pub mod backup;
pub mod civil;
pub mod dupes;
mod handlers;
pub mod import;
pub mod plan;
pub mod stats;

/// Bu kadar ya da daha çok değişiklik içeren toplu işlem önce uyarı ve onay
/// ister (`ui/tools.rs`). "Değişiklik": yazılacak / çıkarılacak şarkı, açılacak
/// liste, yapılacak taşıma; hepsi Spotify'a giden ayrı yazmadır.
pub const BULK_CONFIRM: usize = 50;
