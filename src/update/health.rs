//! Bozuk güncellemeyi geri alma: karar mantığı ve durum dosyası.
//!
//! Burada Velopack'a ya da ağa dokunan hiçbir şey yok: durum + şimdiki sürüm →
//! "ne yapılacak" saf fonksiyonlar ve küçük dosya yardımcıları. Velopack ile
//! konuşan kısım `update.rs`'te (`startup`, `rollback`).
//!
//! Fikir: bir sürüm "sağlıklı" sayılana dek (pencere açıldı ve süreç
//! `HEALTHY_AFTER` boyunca çökmeden yaşadı) her açılış bir "deneme"dir. Sağlıklı
//! olmadan `ROLLBACK_AT_STARTS`. açılışa gelinirse (yani öncekilerden ikisi
//! sağlıklı olamadı) en son sağlıklı sürüme dönülür ve bozuk sürüm kara
//! listeye alınır: daha yeni bir sürüm çıkana kadar yeniden indirilmez.
//!
//! Durum `<önbellek>\update-health.json`'da durur. Bozuk, eksik ya da
//! okunamayan dosya "hiç kayıt yok" sayılır: sağlıklı bir kopya bundan
//! zarar görmez, deneme sayacı yeniden başlar.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use semver::Version;
use serde::{Deserialize, Serialize};

/// Bir sürümün "sağlıklı" sayılması için pencere açıldıktan sonra çökmeden
/// geçmesi gereken süre.
pub const HEALTHY_AFTER: Duration = Duration::from_secs(20);

/// Sağlıklı olmadan bu sayıdaki açılışta (bu açılış dahil) geri dönülür:
/// 3 = iki başarısız deneme, üçüncü açılışta geri dönüş.
pub const ROLLBACK_AT_STARTS: u32 = 3;

const STATE_FILE: &str = "update-health.json";
/// En son sağlıklı sürümün tam paketinin yedeği burada durur (`<önbellek>\rollback`).
pub const BACKUP_DIR: &str = "rollback";

/// Durum dosyası. Her alan isteğe bağlı: eksik ya da tanınmayan alan sorun değil.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// En son sağlıklı çalışmış sürüm.
    pub last_good: Option<String>,
    /// Şu an denenen (henüz sağlıklı sayılmamış) sürüm.
    pub trial_version: Option<String>,
    /// Denenen sürümün sağlıklı olmadan başlama sayısı.
    pub trial_starts: u32,
    /// Geri alınan bozuk sürüm (kara liste): bu sürüm ve altı yeniden
    /// indirilmez, yalnız daha yenisi indirilir.
    pub blocked: Option<String>,
}

/// Açılışta alınacak karar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    /// `to` sürümüne dön; `None` = hedef bilinmiyor (kayıt yok): kaynaktaki
    /// bir önceki sürüm. `bad` kara listeye alınacak sürüm.
    Rollback { to: Option<String>, bad: String },
}

fn parse(v: &str) -> Option<Version> {
    Version::parse(v.trim()).ok()
}

/// Bir açılışı kaydeder. Dönen durum yazılmalı; `Action::Rollback` ise
/// geri dönüş denenmeli.
pub fn on_start(state: &State, current: &str) -> (State, Action) {
    let Some(cur) = parse(current) else {
        // Anlaşılamayan sürüm: dokunma.
        return (state.clone(), Action::None);
    };
    let mut s = state.clone();
    let good = s.last_good.as_deref().and_then(parse);

    if good.as_ref() == Some(&cur) {
        // Sağlıklı olduğu bilinen sürüm: deneme yok.
        s.trial_version = None;
        s.trial_starts = 0;
        return (s, Action::None);
    }

    let same_trial = s.trial_version.as_deref().and_then(parse).as_ref() == Some(&cur);
    if same_trial {
        s.trial_starts = s.trial_starts.saturating_add(1);
    } else {
        s.trial_version = Some(current.trim().to_string());
        s.trial_starts = 1;
    }

    let action = if s.trial_starts >= ROLLBACK_AT_STARTS {
        match good {
            // Sağlıklı sürüm şimdikinden eski: ona dön.
            Some(g) if g < cur => Action::Rollback { to: Some(g.to_string()), bad: current.trim().to_string() },
            // Sağlıklı sürüm şimdikinden yeni ya da eşit değil gibi bir tuhaflık
            // (elle eski sürüm kurulmuş): geri alınacak bir şey yok.
            Some(_) => Action::None,
            // Hiç kayıt yok (ilk kurulum ya da kayıtlı geçmişi olmayan ilk sürüm):
            // kaynaktaki bir önceki sürüm denenir, yoksa vazgeçilir.
            None => Action::Rollback { to: None, bad: current.trim().to_string() },
        }
    } else {
        Action::None
    };
    (s, action)
}

/// Sürüm sağlıklı sayıldı (pencere açık, `HEALTHY_AFTER` çökmeden geçti).
pub fn on_healthy(state: &State, current: &str) -> State {
    let Some(cur) = parse(current) else { return state.clone() };
    let mut s = state.clone();
    s.last_good = Some(current.trim().to_string());
    s.trial_version = None;
    s.trial_starts = 0;
    // Kara listeli sürümden daha yeni bir sürüm sağlıklı çalışıyor: liste artık gereksiz.
    if s.blocked.as_deref().and_then(parse).is_some_and(|b| cur > b) {
        s.blocked = None;
    }
    s
}

/// Uygulama düzgünce kapandı (çökme değil): sağlıklı sayılmadan kapanan bir
/// açılış (hemen kapatıp çıkmak gibi) başarısız deneme sayılmasın.
pub fn on_clean_exit(state: &State, current: &str) -> State {
    let mut s = state.clone();
    let in_trial = s.trial_version.as_deref().and_then(parse) == parse(current);
    if in_trial && s.trial_version.is_some() && s.trial_starts > 0 {
        s.trial_starts -= 1;
    }
    s
}

/// Bu sürüm kara listede mi (indirilmemeli / uygulanmamalı)? Kara listeli sürüm
/// ve altı engellenir; daha yeni sürüm serbest.
pub fn is_blocked(state: &State, candidate: &str) -> bool {
    match (state.blocked.as_deref().and_then(parse), parse(candidate)) {
        (Some(b), Some(c)) => c <= b,
        _ => false,
    }
}

/// Şimdiki sürüm denemede mi (sağlıklı olduğu henüz kanıtlanmadı)?
fn is_proven(state: &State, current: &str) -> bool {
    match (state.last_good.as_deref().and_then(parse), parse(current)) {
        (Some(g), Some(c)) => g == c,
        _ => false,
    }
}

/// Güncelleme indirilmeden önce şimdiki sürümün paketi yedeklenmeli mi?
/// Yalnız sağlıklı olduğu kanıtlanmış sürüm: denemedeki bir sürüm, bilinen
/// sağlıklı sürümün yedeğinin yerine geçmesin.
pub fn should_backup(state: &State, current: &str) -> bool {
    is_proven(state, current)
}

/// Geri dönülecek sürümü kaynaktaki tam paketlerin sürümlerinden seçer.
/// `last_good` biliniyorsa yalnız o (kaynakta yoksa `None`); bilinmiyorsa
/// şimdikinden küçük olanların en büyüğü.
pub fn pick_rollback_target(full_versions: &[String], last_good: Option<&str>, current: &str) -> Option<String> {
    let cur = parse(current)?;
    if let Some(good) = last_good.and_then(parse) {
        if good >= cur {
            return None;
        }
        return full_versions.iter().find(|v| parse(v).as_ref() == Some(&good)).cloned();
    }
    full_versions
        .iter()
        .filter_map(|v| parse(v).map(|p| (p, v)))
        .filter(|(p, _)| *p < cur)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v.clone())
}

// ── Durum dosyası ───────────────────────────────────────────────────────

/// Durum dosyasının yolu (önbellek klasöründe; sınama koşusunda kum havuzunda).
pub fn state_path() -> Option<PathBuf> {
    Some(crate::engine::settings_cache_dir()?.join(STATE_FILE))
}

/// Yedek klasörü (`<önbellek>\rollback`).
pub fn backup_dir() -> Option<PathBuf> {
    Some(crate::engine::settings_cache_dir()?.join(BACKUP_DIR))
}

/// Dosyayı okur. Yok, boş ya da bozuksa varsayılan (kayıt yok).
pub fn load_from(path: &Path) -> State {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Dosyayı yazar (önce geçici dosya, sonra yer değiştirme: yarım yazı kalmasın).
pub fn save_to(path: &Path, state: &State) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    fs::rename(&tmp, path)
}

// ── Paket yedeği ────────────────────────────────────────────────────────

/// Paket dosya adı bu sürümün tam paketi mi? Velopack adı
/// `<Kimlik>-<sürüm>-<kanal>-full.nupkg` (eski biçimde kanal yok); `1.2.3`,
/// `1.2.30` ya da `1.2.3-beta.1` ile karışmaz.
pub fn package_matches_version(file_name: &str, version: &str) -> bool {
    let name = file_name.to_ascii_lowercase();
    let Some(stem) = name.strip_suffix("-full.nupkg") else { return false };
    let want = format!("-{}", version.trim().to_ascii_lowercase());
    if stem.ends_with(&want) {
        return true;
    }
    // Kanal eki: "<kimlik>-<sürüm>-<kanal>"
    stem.rsplit_once('-').is_some_and(|(id_ver, _channel)| id_ver.ends_with(&want))
}

/// Klasördeki, bu sürümün tam paketi.
pub fn find_package(dir: &Path, version: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| package_matches_version(n, version)))
        .map(|e| e.path())
        .next()
}

/// `packages` klasöründeki şimdiki sürümün tam paketini `rollback` klasörüne
/// yedekler; klasördeki diğer dosyalar silinir (hep tek yedek). Zaten aynı
/// boyutta yedek varsa kopyalamaz. `Ok(true)`: yedek hazır; `Ok(false)`: paket yok.
pub fn backup_package(packages_dir: &Path, rollback_dir: &Path, version: &str) -> io::Result<bool> {
    let Some(src) = find_package(packages_dir, version) else { return Ok(false) };
    let name = src.file_name().ok_or_else(|| io::Error::other("package has no file name"))?.to_owned();
    fs::create_dir_all(rollback_dir)?;
    let dst = rollback_dir.join(&name);
    let up_to_date = fs::metadata(&dst).ok().zip(fs::metadata(&src).ok()).is_some_and(|(a, b)| a.len() == b.len());
    if !up_to_date {
        let tmp = rollback_dir.join(format!("{}.partial", name.to_string_lossy()));
        fs::copy(&src, &tmp)?;
        fs::rename(&tmp, &dst)?;
    }
    // Eski yedekleri (başka sürümler, yarım dosyalar) temizle.
    for entry in fs::read_dir(rollback_dir)?.flatten() {
        if entry.file_name() != name {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(true)
}

/// Yedeği `packages` klasörüne koyar ve oradaki bütün diğer paketleri (bozuk
/// sürümün tam paketi, delta'lar, yarım dosyalar) siler: bozuk sürüm
/// "bekleyen güncelleme" olarak yeniden uygulanmasın. Paketin dosya adını döner.
pub fn stage_backup(backup: &Path, packages_dir: &Path) -> io::Result<String> {
    let name = backup
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("backup has no file name"))?
        .to_string();
    fs::create_dir_all(packages_dir)?;
    let dst = packages_dir.join(&name);
    let tmp = packages_dir.join(format!("{name}.partial"));
    fs::copy(backup, &tmp)?;
    for entry in fs::read_dir(packages_dir)?.flatten() {
        let path = entry.path();
        let file = entry.file_name();
        let file = file.to_string_lossy().to_ascii_lowercase();
        let ours = file == format!("{}.partial", name.to_ascii_lowercase());
        if !ours && (file.ends_with(".nupkg") || file.ends_with(".partial")) {
            let _ = fs::remove_file(path);
        }
    }
    fs::rename(&tmp, &dst)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(last_good: Option<&str>, trial: Option<(&str, u32)>, blocked: Option<&str>) -> State {
        State {
            last_good: last_good.map(String::from),
            trial_version: trial.map(|(v, _)| v.to_string()),
            trial_starts: trial.map_or(0, |(_, n)| n),
            blocked: blocked.map(String::from),
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openify-health-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn first_install_starts_a_trial_and_becomes_good() {
        let (s, a) = on_start(&State::default(), "1.0.0");
        assert_eq!(a, Action::None);
        assert_eq!(s, st(None, Some(("1.0.0", 1)), None));
        let s = on_healthy(&s, "1.0.0");
        assert_eq!(s, st(Some("1.0.0"), None, None));
        // Sağlıklı sürümün sonraki açılışları değişiklik yazmaz.
        let (again, a) = on_start(&s, "1.0.0");
        assert_eq!((again, a), (s, Action::None));
    }

    #[test]
    fn healthy_update_replaces_last_good() {
        let s = st(Some("1.0.0"), None, None);
        let (s, a) = on_start(&s, "1.1.0");
        assert_eq!(a, Action::None);
        assert_eq!(s, st(Some("1.0.0"), Some(("1.1.0", 1)), None));
        let s = on_healthy(&s, "1.1.0");
        assert_eq!(s, st(Some("1.1.0"), None, None));
    }

    #[test]
    fn two_failed_trials_roll_back_on_the_third_start() {
        let mut s = st(Some("1.0.0"), None, None);
        let (n, a) = on_start(&s, "1.1.0");
        assert_eq!(a, Action::None, "1. açılış");
        s = n;
        let (n, a) = on_start(&s, "1.1.0");
        assert_eq!(a, Action::None, "2. açılış (1 başarısız deneme)");
        assert_eq!(n.trial_starts, 2);
        s = n;
        let (n, a) = on_start(&s, "1.1.0");
        assert_eq!(a, Action::Rollback { to: Some("1.0.0".into()), bad: "1.1.0".into() });
        assert_eq!(n.trial_starts, 3);
    }

    #[test]
    fn healthy_in_between_resets_the_counter() {
        let s = st(Some("1.0.0"), Some(("1.1.0", 2)), None);
        let s = on_healthy(&s, "1.1.0");
        let (s, a) = on_start(&s, "1.1.0");
        assert_eq!(a, Action::None);
        assert_eq!(s.trial_starts, 0);
    }

    #[test]
    fn a_new_version_mid_trial_restarts_the_count() {
        let s = st(Some("1.0.0"), Some(("1.1.0", 2)), None);
        let (s, a) = on_start(&s, "1.2.0");
        assert_eq!(a, Action::None);
        assert_eq!(s, st(Some("1.0.0"), Some(("1.2.0", 1)), None));
    }

    #[test]
    fn rollback_blocks_the_bad_version_until_a_newer_one() {
        // Geri dönüşten sonraki durum: sağlıklı 1.0.0, kara liste 1.1.0.
        let s = st(Some("1.0.0"), None, Some("1.1.0"));
        assert!(is_blocked(&s, "1.1.0"));
        assert!(is_blocked(&s, "1.0.5"), "kara listeli sürümün altı da engelli");
        assert!(!is_blocked(&s, "1.1.1"), "daha yeni sürüm serbest");
        assert!(!is_blocked(&s, "2.0.0"));
        assert!(!is_blocked(&State::default(), "1.1.0"));
        // Eski sürüm açılınca (sağlıklı) kara liste durur.
        let (s2, a) = on_start(&s, "1.0.0");
        assert_eq!((s2.clone(), a), (s.clone(), Action::None));
        assert_eq!(on_healthy(&s2, "1.0.0").blocked.as_deref(), Some("1.1.0"));
        // Daha yeni sürüm sağlıklı çalışınca liste kalkar.
        let (n, _) = on_start(&s, "1.2.0");
        assert_eq!(on_healthy(&n, "1.2.0").blocked, None);
    }

    #[test]
    fn unknown_history_rolls_back_to_the_previous_release() {
        let mut s = State::default();
        for i in 1..=3 {
            let (n, a) = on_start(&s, "1.0.0");
            s = n;
            if i < 3 {
                assert_eq!(a, Action::None);
            } else {
                assert_eq!(a, Action::Rollback { to: None, bad: "1.0.0".into() });
            }
        }
    }

    #[test]
    fn no_rollback_when_last_good_is_not_older() {
        // Elle eski sürüm kurulmuş: şimdiki sürüm kayıtlı sağlıklıdan eski.
        let s = st(Some("1.2.0"), Some(("1.0.0", 5)), None);
        let (_, a) = on_start(&s, "1.0.0");
        assert_eq!(a, Action::None);
    }

    #[test]
    fn clean_exit_undoes_the_start_of_an_unproven_version() {
        let s = st(Some("1.0.0"), Some(("1.1.0", 2)), None);
        assert_eq!(on_clean_exit(&s, "1.1.0").trial_starts, 1);
        assert_eq!(on_clean_exit(&st(Some("1.0.0"), Some(("1.1.0", 0)), None), "1.1.0").trial_starts, 0);
        // Sağlıklı sürümde ya da başka sürümün denemesinde değişmez.
        let good = st(Some("1.1.0"), None, None);
        assert_eq!(on_clean_exit(&good, "1.1.0"), good);
        assert_eq!(on_clean_exit(&s, "1.2.0"), s);
        // İki hızlı kapatma + sağlıklı olmayan açılış geri dönüşe yol açmaz.
        let mut s = st(Some("1.0.0"), None, None);
        for _ in 0..5 {
            let (n, a) = on_start(&s, "1.1.0");
            assert_eq!(a, Action::None);
            s = on_clean_exit(&n, "1.1.0");
        }
    }

    #[test]
    fn semver_order_is_numeric_not_textual() {
        // Metin sırasıyla "1.9.0" > "1.10.0"; semver'de tersi.
        let s = st(Some("1.9.0"), Some(("1.10.0", 2)), None);
        let (_, a) = on_start(&s, "1.10.0");
        assert_eq!(a, Action::Rollback { to: Some("1.9.0".into()), bad: "1.10.0".into() });
        assert!(is_blocked(&st(None, None, Some("1.10.0")), "1.9.0"));
        assert!(!is_blocked(&st(None, None, Some("1.9.0")), "1.10.0"));
        // Ön sürüm, aynı numaralı kararlı sürümden eski.
        assert!(is_blocked(&st(None, None, Some("1.2.0")), "1.2.0-beta.1"));
        assert!(!is_blocked(&st(None, None, Some("1.2.0-beta.1")), "1.2.0"));
        let s = st(Some("1.2.0-beta.1"), Some(("1.2.0", 2)), None);
        assert!(matches!(on_start(&s, "1.2.0").1, Action::Rollback { .. }));
    }

    #[test]
    fn unparsable_versions_never_trigger_anything() {
        let s = st(Some("1.0.0"), Some(("1.1.0", 9)), None);
        assert_eq!(on_start(&s, "bozuk"), (s.clone(), Action::None));
        assert_eq!(on_healthy(&s, ""), s);
        // Durumdaki bozuk sürüm "kayıt yok" gibi davranır, çökmez.
        let (n, a) = on_start(&st(Some("abc"), None, Some("???")), "1.0.0");
        assert_eq!(a, Action::None);
        assert_eq!(n.trial_starts, 1);
        assert!(!is_blocked(&st(None, None, Some("???")), "1.0.0"));
        assert!(!is_blocked(&st(None, None, Some("1.0.0")), "bozuk"));
    }

    #[test]
    fn backup_only_for_proven_versions() {
        assert!(should_backup(&st(Some("1.0.0"), None, None), "1.0.0"));
        assert!(!should_backup(&st(Some("1.0.0"), Some(("1.1.0", 1)), None), "1.1.0"));
        assert!(!should_backup(&State::default(), "1.0.0"));
        assert!(!should_backup(&st(Some("1.0.0"), None, None), "yok"));
    }

    #[test]
    fn rollback_target_comes_from_the_feed() {
        let feed: Vec<String> = ["1.0.0", "1.1.0", "1.2.0", "1.10.0"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_rollback_target(&feed, Some("1.1.0"), "1.10.0").as_deref(), Some("1.1.0"));
        assert_eq!(pick_rollback_target(&feed, Some("0.9.0"), "1.10.0"), None, "kaynakta yok");
        assert_eq!(pick_rollback_target(&feed, Some("1.10.0"), "1.10.0"), None, "eski değil");
        assert_eq!(pick_rollback_target(&feed, None, "1.10.0").as_deref(), Some("1.2.0"));
        assert_eq!(pick_rollback_target(&feed, None, "1.0.0"), None);
        assert_eq!(pick_rollback_target(&[], None, "1.0.0"), None);
        assert_eq!(pick_rollback_target(&feed, None, "bozuk"), None);
    }

    #[test]
    fn corrupt_missing_or_partial_state_file_is_empty() {
        let dir = temp("state");
        let path = dir.join("update-health.json");
        assert_eq!(load_from(&path), State::default(), "dosya yok");
        fs::write(&path, "").unwrap();
        assert_eq!(load_from(&path), State::default(), "boş");
        fs::write(&path, "{ bozuk json").unwrap();
        assert_eq!(load_from(&path), State::default(), "bozuk");
        fs::write(&path, "[1,2,3]").unwrap();
        assert_eq!(load_from(&path), State::default(), "yanlış biçim");
        fs::write(&path, r#"{"last_good":"1.0.0","bilinmeyen":7}"#).unwrap();
        assert_eq!(load_from(&path), st(Some("1.0.0"), None, None), "eksik ve fazla alan");
        fs::write(&path, r#"{"trial_starts":"çok"}"#).unwrap();
        assert_eq!(load_from(&path), State::default(), "yanlış türde alan");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_file_round_trips() {
        let dir = temp("roundtrip");
        let path = dir.join("alt").join("update-health.json");
        let s = st(Some("1.0.0"), Some(("1.1.0", 2)), Some("1.1.0"));
        save_to(&path, &s).unwrap();
        assert_eq!(load_from(&path), s);
        save_to(&path, &State::default()).unwrap();
        assert_eq!(load_from(&path), State::default());
        assert!(!path.with_extension("json.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_names_match_their_own_version_only() {
        assert!(package_matches_version("Openify-1.2.3-win-full.nupkg", "1.2.3"));
        assert!(package_matches_version("openify-1.2.3-win-full.nupkg", "1.2.3"));
        assert!(package_matches_version("Openify-1.2.3-full.nupkg", "1.2.3"), "kanalsız eski biçim");
        assert!(package_matches_version("Openify-1.2.3-beta.1-win-full.nupkg", "1.2.3-beta.1"));
        assert!(!package_matches_version("Openify-1.2.30-win-full.nupkg", "1.2.3"));
        assert!(!package_matches_version("Openify-1.2.3-beta.1-win-full.nupkg", "1.2.3"));
        assert!(!package_matches_version("Openify-1.2.3-win-delta.nupkg", "1.2.3"));
        assert!(!package_matches_version("Openify-1.2.3-win-full.nupkg.partial", "1.2.3"));
        assert!(!package_matches_version("releases.win.json", "1.2.3"));
    }

    #[test]
    fn backup_keeps_a_single_copy_and_stage_cleans_packages() {
        let dir = temp("backup");
        let packages = dir.join("packages");
        let backup = dir.join("rollback");
        fs::create_dir_all(&packages).unwrap();
        fs::write(packages.join("Openify-1.0.0-win-full.nupkg"), b"eski-iyi").unwrap();

        // Paket yoksa yedek yok.
        assert!(!backup_package(&packages, &backup, "9.9.9").unwrap());
        assert!(backup_package(&packages, &backup, "1.0.0").unwrap());
        assert_eq!(fs::read(backup.join("Openify-1.0.0-win-full.nupkg")).unwrap(), b"eski-iyi");
        // İkinci yedek zararsız (aynı boyut: yeniden kopyalanmaz).
        assert!(backup_package(&packages, &backup, "1.0.0").unwrap());

        // Yeni sürüm yedeklenince eskisi silinir; hep tek dosya.
        fs::write(packages.join("Openify-1.1.0-win-full.nupkg"), b"yeni").unwrap();
        fs::write(backup.join("artik.partial"), b"x").unwrap();
        assert!(backup_package(&packages, &backup, "1.1.0").unwrap());
        let names: Vec<_> = fs::read_dir(&backup).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("Openify-1.1.0-win-full.nupkg")]);

        // Geri dönüş: yedek geri konur, bozuk sürüm paketi ve delta'lar silinir.
        let saved = backup.join("Openify-1.1.0-win-full.nupkg");
        fs::write(packages.join("Openify-1.2.0-win-full.nupkg"), b"bozuk").unwrap();
        fs::write(packages.join("Openify-1.2.0-win-delta.nupkg"), b"delta").unwrap();
        fs::write(packages.join("Openify-1.2.0-win-full.partial"), b"yarim").unwrap();
        fs::write(packages.join(".betaId"), b"id").unwrap();
        let name = stage_backup(&saved, &packages).unwrap();
        assert_eq!(name, "Openify-1.1.0-win-full.nupkg");
        let mut left: Vec<String> =
            fs::read_dir(&packages).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec![".betaId".to_string(), "Openify-1.1.0-win-full.nupkg".to_string()]);
        assert_eq!(fs::read(packages.join(&name)).unwrap(), b"yeni");
        assert!(saved.exists(), "yedek yerinde kalır (uygulama başarısız olursa yeniden denenir)");
        let _ = fs::remove_dir_all(&dir);
    }
}
