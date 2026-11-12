//! Dinleme çekirdeği, arayüz tarafı: radyo (menü girdisi, başlatma), yerel
//! şarkı gizleme (menü girdileri, soluk satır, Ayarlar'daki liste) ve gizli
//! oturum (profil menüsü, palet, başlık çubuğu simgesi).
//!
//! - **Radyo**: sağ tık menüsünde (şarkı, sanatçı, albüm, liste) "Radyoyu
//!   başlat" ve palette "Çalan şarkının radyosunu başlat"; motora tek komut
//!   (`Command::StartRadio`), gerisi motorda (`engine/listening.rs`). Radyo
//!   çalarken sıra başlığı "Sıradaki: <ad> radyosu" olur (`QueueView::radio`).
//! - **Gizleme**: "Bu şarkıyı gizle" (bağlam başına: açık liste / albüm) ve
//!   "Hiçbir yerde çalma" (genel). Gizli şarkının satırı soluk görünür, menüde
//!   "Göster" yazar; Ayarlar → Karıştırma ve sıra'da liste var. Yerel: Spotify
//!   hesabına hiçbir şey yazılmaz. Dizin motordan gelir (`LibraryEvent::Hidden`);
//!   arayüz kendi kopyasını eylemde hemen günceller (satır motoru beklemeden
//!   solar), motorun cevabı onu doğrular.
//! - **Gizli oturum**: açıkken Son çalınanlar (`Shell::note_played`) ve yerel
//!   istatistik (`stats::set_private_session`; hem arayüz hem motor bakar)
//!   yazılmaz, başlık çubuğunda simge görünür. Kaldığı yer yine kaydedilir.
//!   Oturum UYGULAMA KAPANINCA BİTER (kalıcı değil; ayar dosyasına yazılmaz):
//!   unutulan bir gizli oturum kullanıcıyı sessizce geçmişsiz bırakmasın.
//!   Openify Spotify'a zaten dinleme bildirmez (bkz. ARCHITECTURE), bu yüzden
//!   yazılar yalnız "Openify'ın kendi geçmişine yazmaz" der.
//!
//! Sınama kancaları (yalnız sınama koşusunda; ağa çıkılmaz):
//! `OPENIFY_TEST_PRIVATE=1` gizli oturum açık başlar;
//! `OPENIFY_TEST_RADIO_JSON=<dosya>` (ör. `src\engine\fixtures\radio_apollo.json`)
//! radyo yanıtından kurulu bir radyo "çalıyor" gösterilir (sahte çalma) ve sıra
//! paneli radyo başlığını taşır; gizlenen şarkılar için demo kütüphanenin
//! `library.db`'sinde (`seed_demo_library`) kayıt var.

use std::sync::Arc;

use gpui::{AnyElement, ClickEvent, Context, ElementId, FontWeight, Window, div, prelude::*, px};

use super::{
    Shell,
    components::{Tooltip, icon_button, secondary_button, switch},
    manage::{Act, Entry, ListKey},
    settings::{row_text, setting_row},
    text::line_with_tip,
    theme::*,
};
use crate::engine::{
    self, Command, HiddenList, HiddenSong, LibraryCommand, RadioInfo, TrackRow, tools::stats,
};
use crate::{t, tf, tp};

/// Gizli şarkı satırının donukluğu: okunur kalır ama dikkat çekmez.
const HIDDEN_ROW_OPACITY: f32 = 0.42;
/// Ayarlardaki uzun açıklamanın genişliği (Ayarlar sütunu 720 px; kenarlık payı).
const SETTINGS_TEXT_WIDTH: f32 = 700.;
/// Ayarlardaki listede en çok bu kadar satır (gerisi "ve N tane daha").
const SETTINGS_ROWS: usize = 30;

pub(super) struct ListeningUi {
    /// Motorun son bildirdiği gizli şarkılar (+ eylemde hemen güncellenen).
    pub hidden: Arc<HiddenList>,
    /// Gizli oturum açık mı (uygulama kapanınca biter).
    pub private: bool,
}

impl ListeningUi {
    pub fn new() -> Self {
        Self {
            hidden: Arc::new(HiddenList::default()),
            private: false,
        }
    }
}

/// Satırın bağlam anahtarı: kütüphane listesi / albüm ya da albüm sayfası.
/// Öteki listelerde (çok dinlenenler, arama) bağlama özgü gizleme yok.
pub(super) fn list_context(list: &ListKey) -> Option<&str> {
    match list {
        ListKey::Collection(id) => Some(id.key()),
        ListKey::Page(key) => key.strip_prefix("album:"),
    }
}

/// Seçilen satırlara göre menüde hangi gizleme girdileri görünür.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HideChoices {
    /// "Göster": hepsi gizli.
    pub show: bool,
    /// "Bu şarkıyı gizle": bağlam var ve hepsi gizli değil.
    pub hide_here: bool,
    /// "Hiçbir yerde çalma": hepsi zaten genel gizli değil.
    pub hide_everywhere: bool,
}

/// Saf karar (sınanır): `rows` seçilen şarkı adresleri, `context` listenin anahtarı.
pub(super) fn hide_choices(
    hidden: &HiddenList,
    context: Option<&str>,
    uris: &[&str],
) -> HideChoices {
    let all_hidden = !uris.is_empty() && uris.iter().all(|u| hidden.hides(context, u));
    let all_global = !uris.is_empty() && uris.iter().all(|u| hidden.hidden_everywhere(u));
    HideChoices {
        show: all_hidden,
        hide_here: !all_hidden && context.is_some(),
        hide_everywhere: !all_global,
    }
}

/// `rows` içinden henüz `context`te (None: her yerde) gizli olmayanlar.
fn not_yet_hidden(hidden: &HiddenList, context: Option<&str>, rows: &[TrackRow]) -> Vec<TrackRow> {
    rows.iter()
        .filter(|r| match context {
            Some(c) => !hidden.hides(Some(c), &r.uri),
            None => !hidden.hidden_everywhere(&r.uri),
        })
        .cloned()
        .collect()
}

/// Son çalınanlar'a yazma kararı ve yazımı: gizli oturumda hiçbir şey yazılmaz.
/// Yazıldıysa (listenin başı zaten bu değilse) `true`.
pub(super) fn note_recent(private: bool, recent: &mut Vec<String>, key: &str) -> bool {
    if private || recent.first().map(String::as_str) == Some(key) {
        return false;
    }
    crate::settings::note_played(recent, key);
    true
}

fn song_of(row: &TrackRow, context: Option<&str>) -> HiddenSong {
    HiddenSong {
        uri: row.uri.clone(),
        context: context.map(str::to_string),
        name: row.name.clone(),
        artists: row.artists.clone(),
        hidden_at: now_ms(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn entry(icon: &'static str, label: String, act: Act) -> Entry {
    Entry::Item {
        icon,
        label,
        shortcut: None,
        danger: false,
        act: Some(act),
        sub: Vec::new(),
    }
}

impl Shell {
    // ── Gizlenen şarkılar ───────────────────────────────────────────────

    /// Motor güncel gizli şarkı dizinini bildirdi.
    pub(super) fn on_hidden(&mut self, list: Arc<HiddenList>) {
        self.listen.hidden = list;
    }

    /// Bu satır soluk mu (bu listede ya da her yerde gizli)?
    pub(super) fn row_hidden(&self, list: &ListKey, uri: &str) -> bool {
        !self.listen.hidden.filters_nothing() && self.listen.hidden.hides(list_context(list), uri)
    }

    /// Satır soluk çizilirken uygulanacak donukluk.
    pub(super) fn hidden_opacity() -> f32 {
        HIDDEN_ROW_OPACITY
    }

    /// Şarkı menüsündeki gizleme girdileri (tek ya da çok seçim).
    pub(super) fn hide_entries(&self, list: &ListKey, rows: &[TrackRow]) -> Vec<Entry> {
        let context = list_context(list);
        let uris: Vec<&str> = rows.iter().map(|r| r.uri.as_str()).collect();
        let choices = hide_choices(&self.listen.hidden, context, &uris);
        let single = rows.len() == 1;
        let mut out = Vec::new();
        if choices.hide_here {
            let label = if single {
                t!(MenuHideSong).to_string()
            } else {
                tf!(MenuHideSongs, rows.len())
            };
            out.push(entry(
                "eye-off",
                label,
                Act::HideSongs {
                    rows: rows.to_vec(),
                    context: context.map(str::to_string),
                },
            ));
        }
        if choices.hide_everywhere {
            // Tek ve çok seçimde aynı yazı: "Hiçbir yerde çalma" sayı taşımaz.
            out.push(entry(
                "ban",
                t!(MenuNeverPlayAnywhere).to_string(),
                Act::HideSongs {
                    rows: rows.to_vec(),
                    context: None,
                },
            ));
        }
        if choices.show {
            let label = if single {
                t!(MenuShowSong).to_string()
            } else {
                tf!(MenuShowSongs, rows.len())
            };
            // "Göster" ilk sırada: gizliyken en olası eylem.
            out.insert(
                0,
                entry(
                    "eye",
                    label,
                    Act::ShowSongs {
                        rows: rows.to_vec(),
                        context: context.map(str::to_string),
                    },
                ),
            );
        }
        out
    }

    /// Listeyi baştan çalarken ilk şarkı: gizli olanlar atlanır (kullanıcı
    /// satıra çift tıklayınca değil, "Çal" düğmesiyle başlatınca; satır seçimi
    /// her zaman çalar). Hepsi gizliyse 0.
    pub(super) fn first_shown(&self, list: &ListKey) -> usize {
        let Some(tracks) = self.list_tracks(list) else {
            return 0;
        };
        tracks
            .iter()
            .position(|t| !self.row_hidden(list, &t.uri))
            .unwrap_or(0)
    }

    /// Seçili kütüphane listesini baştan çal (ilk gizlenmemiş şarkıdan).
    pub(super) fn play_from_start(&mut self, cx: &mut Context<Self>) {
        let index = self
            .library
            .selected
            .clone()
            .map_or(0, |id| self.first_shown(&ListKey::Collection(id)));
        self.play_row(index, cx);
    }

    /// Şarkıları gizle (`context`: bu liste / albüm; `None`: hiçbir yerde).
    pub(super) fn hide_songs(
        &mut self,
        rows: Vec<TrackRow>,
        context: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let rows = not_yet_hidden(&self.listen.hidden, context.as_deref(), &rows);
        if rows.is_empty() {
            return;
        }
        let songs: Vec<HiddenSong> = rows
            .iter()
            .map(|r| song_of(r, context.as_deref()))
            .collect();
        // Satır hemen solar; motorun cevabı (aynı dizin) onu doğrular.
        let mut list = (*self.listen.hidden).clone();
        for s in &songs {
            list = list.with(s.clone());
        }
        self.listen.hidden = Arc::new(list);
        self.engine
            .send(Command::Library(LibraryCommand::SetHidden {
                songs,
                on: true,
            }));
        let n = rows.len();
        let text = if context.is_some() {
            tp!(ToastHiddenHere, n)
        } else {
            tp!(ToastHiddenEverywhere, n)
        };
        self.show_toast(text, false, false, cx);
    }

    /// Gizlemeyi kaldır: bu bağlamdaki kayıt ve (varsa) genel kayıt.
    pub(super) fn show_songs(
        &mut self,
        rows: Vec<TrackRow>,
        context: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let mut songs = Vec::new();
        for r in &rows {
            if let Some(c) = context.as_deref()
                && self.listen.hidden.hidden_in(c, &r.uri)
            {
                songs.push(song_of(r, Some(c)));
            }
            if self.listen.hidden.hidden_everywhere(&r.uri) {
                songs.push(song_of(r, None));
            }
        }
        self.show_entries(songs, cx);
    }

    /// Ayarlardaki listeden tek kayıt.
    fn show_entry(&mut self, song: HiddenSong, cx: &mut Context<Self>) {
        self.show_entries(vec![song], cx);
    }

    fn show_entries(&mut self, songs: Vec<HiddenSong>, cx: &mut Context<Self>) {
        if songs.is_empty() {
            return;
        }
        let mut list = (*self.listen.hidden).clone();
        for s in &songs {
            list = list.without(&s.uri, s.context.as_deref());
        }
        self.listen.hidden = Arc::new(list);
        let n = songs
            .iter()
            .map(|s| s.uri.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        self.engine
            .send(Command::Library(LibraryCommand::SetHidden {
                songs,
                on: false,
            }));
        self.show_toast(tp!(ToastHiddenShown, n), false, false, cx);
    }

    /// Ayarlar → Karıştırma ve sıra: başlık satırı, kayıtlar (en yeni önde).
    pub(super) fn hidden_settings_rows(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let accent = self.accent().text;
        let hidden = self.listen.hidden.clone();
        // Başlık satırı: açıklama uzun (üç cümle); genişliği sınırlı kutuda kendiliğinden alta kayar.
        let header = div()
            .py(px(S3))
            .border_t_1()
            .border_color(veil(LINE))
            .flex()
            .flex_col()
            .gap(px(S0))
            .child(
                div()
                    .type_role(T_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text())
                    .child(t!(RowHidden)),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(SETTINGS_TEXT_WIDTH))
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(t!(RowHiddenDesc)),
            )
            .into_any_element();
        let mut rows = vec![header];
        if hidden.is_empty() {
            rows.push(
                div()
                    .pb(px(S3))
                    .type_role(T_SECONDARY)
                    .text_color(label())
                    .child(t!(HiddenEmpty))
                    .into_any_element(),
            );
            return rows;
        }
        for (i, song) in hidden.entries().iter().take(SETTINGS_ROWS).enumerate() {
            let (stop, look) = self.stop_look(
                format!("show-hidden-{i}"),
                super::tab::PAGE,
                true,
                window,
                cx,
            );
            let scope = match &song.context {
                None => t!(HiddenEverywhere).to_string(),
                Some(key) => match engine::CollectionId::from_key(key)
                    .map(|id| self.library.title(&id))
                    .filter(|n| !n.is_empty())
                {
                    Some(name) => tf!(HiddenInList, name),
                    None => t!(HiddenInUnknown).to_string(),
                },
            };
            let name = if song.name.is_empty() {
                song.uri.clone()
            } else {
                song.name.clone()
            };
            let detail = if song.artists.is_empty() {
                scope
            } else {
                format!("{} · {}", song.artists, scope)
            };
            let target = song.clone();
            let show = secondary_button(
                ElementId::Name(format!("show-hidden-{i}").into()),
                "eye",
                t!(HiddenShow),
                look,
                accent,
            )
            .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.leave_button(ev, window);
                this.show_entry(target.clone(), cx);
            }))
            .into_any_element();
            let left = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .type_role(T_BODY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text())
                        .child(
                            line_with_tip(ElementId::Name(format!("hidden-name-{i}").into()), name)
                                .flex()
                                .min_w_0(),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .type_role(T_SECONDARY)
                        .text_color(muted())
                        .child(
                            line_with_tip(
                                ElementId::Name(format!("hidden-detail-{i}").into()),
                                detail,
                            )
                            .flex()
                            .min_w_0(),
                        ),
                );
            rows.push(setting_row(false, left, show));
        }
        if hidden.len() > SETTINGS_ROWS {
            rows.push(
                div()
                    .pt(px(S3))
                    .type_role(T_SECONDARY)
                    .text_color(label())
                    .child(tf!(HiddenMore, hidden.len() - SETTINGS_ROWS))
                    .into_any_element(),
            );
        }
        rows
    }

    // ── Radyo ───────────────────────────────────────────────────────────

    /// Kart menüsünün radyo tohumu ve görünen adı (Beğenilenler ve klasörde yok).
    pub(super) fn card_radio_seed(
        &self,
        target: &super::manage::CardTarget,
    ) -> Option<(String, String)> {
        use super::manage::CardTarget;
        match target {
            CardTarget::Collection { id, title } if id != &engine::CollectionId::Liked => {
                Some((id.key().to_string(), title.clone()))
            }
            CardTarget::Artist(uri) => Some((
                uri.clone(),
                self.known_artist(uri).map(|a| a.name).unwrap_or_default(),
            )),
            CardTarget::Album { uri, item } => Some((
                uri.clone(),
                item.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| {
                    self.library
                        .title(&engine::CollectionId::Album(uri.clone()))
                }),
            )),
            _ => None,
        }
    }

    /// Menü girdisi: "Radyoyu başlat" (`seed`: şarkı / sanatçı / albüm / liste adresi).
    pub(super) fn radio_entry(seed: &str, title: &str) -> Option<Entry> {
        engine::radio_seed_ok(seed).then(|| {
            entry(
                "radio",
                t!(MenuStartRadio).into(),
                Act::StartRadio {
                    seed: seed.to_string(),
                    title: title.to_string(),
                },
            )
        })
    }

    /// Radyoyu başlat: motora tek komut; hazır değilse (giriş yok, çevrimdışı) ne
    /// olduğu söylenir (`ready_to_play`).
    pub(super) fn start_radio(&mut self, seed: String, title: String, cx: &mut Context<Self>) {
        if !self.ready_to_play(cx) {
            return;
        }
        self.state.notice = None;
        self.engine.send(Command::StartRadio { seed, title });
        self.show_toast(t!(ToastRadioStarting).into(), false, false, cx);
    }

    /// Palet: çalan şarkının radyosu.
    pub(super) fn start_radio_from_current(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.state.track.clone() else {
            return;
        };
        self.start_radio(track.uri, track.name, cx);
    }

    /// Sıra başlığı: radyo çalıyorsa "Sıradaki: <ad> radyosu".
    pub(super) fn radio_head(info: &RadioInfo) -> String {
        if info.title.trim().is_empty() {
            t!(QueueNextFromRadioPlain).into()
        } else {
            tf!(QueueNextFromRadio, info.title)
        }
    }

    // ── Gizli oturum ────────────────────────────────────────────────────

    pub(super) fn private_session(&self) -> bool {
        self.listen.private
    }

    /// Gizli oturumu aç / kapat (profil menüsü, palet, başlık çubuğu simgesi,
    /// Ayarlar). Kalıcı değil: uygulama kapanınca biter.
    pub(super) fn set_private_session(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.listen.private == on {
            return;
        }
        self.listen.private = on;
        stats::set_private_session(on);
        self.show_toast(
            if on {
                t!(ToastPrivateOn)
            } else {
                t!(ToastPrivateOff)
            }
            .into(),
            false,
            false,
            cx,
        );
        cx.notify();
    }

    pub(super) fn toggle_private_session(&mut self, cx: &mut Context<Self>) {
        self.set_private_session(!self.listen.private, cx);
    }

    /// Profil menüsündeki satır: menüyü kapatıp aç / kapa.
    pub(super) fn toggle_private_from_menu(&mut self, cx: &mut Context<Self>) {
        self.sui.profile_open = false;
        self.toggle_private_session(cx);
    }

    /// Başlık çubuğundaki küçük simge (yalnız gizli oturumda); tıklayınca kapatır.
    pub(super) fn render_private_badge(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.listen.private {
            return None;
        }
        let (stop, look) = self.stop_look("private-badge", super::tab::FRIENDS, true, window, cx);
        Some(
            div()
                .mr(px(S2))
                .child(
                    icon_button("private-badge", "eye-off", look, self.accent().text)
                        .when_some(stop, |el, h| el.track_focus(&h))
                        .tooltip(Tooltip::text(t!(PrivateBadgeTip), None))
                        .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                            this.leave_button(ev, window);
                            this.set_private_session(false, cx);
                        })),
                )
                .into_any_element(),
        )
    }

    /// Ayarlar → Davranış: gizli oturum anahtarı (kalıcı değil, `Settings`e yazılmaz).
    pub(super) fn private_setting_row(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.listen.private;
        let (stop, look) = self.stop_look("private-session", super::tab::PAGE, true, window, cx);
        let control = switch("private-session", on, look, self.accent().text)
            .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
            .on_click(cx.listener(move |this, _, _, cx| this.set_private_session(!on, cx)))
            .into_any_element();
        setting_row(
            false,
            row_text(t!(RowPrivate), t!(RowPrivateDesc), None),
            control,
        )
    }

    // ── Sınama kancaları ────────────────────────────────────────────────

    /// `OPENIFY_TEST_PRIVATE=1` ve `OPENIFY_TEST_RADIO_JSON` (yalnız sınama koşusunda).
    pub(super) fn init_listening_fixtures(&mut self) {
        let testing = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
            || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some();
        if !testing {
            return;
        }
        if std::env::var_os("OPENIFY_TEST_PRIVATE").is_some() {
            self.listen.private = true;
            stats::set_private_session(true);
        }
        let Some(value) = engine::radio_test_fixture() else {
            return;
        };
        let station = engine::parse_radio(&value);
        let rows: Vec<TrackRow> = station.tracks.into_iter().filter_map(|t| t.row).collect();
        let Some(first) = rows.first().cloned() else {
            return;
        };
        let queue_entry = |t: &TrackRow| engine::QueueEntry {
            uri: t.uri.clone(),
            track: Some(t.clone()),
            source: engine::QueueSource::Context,
        };
        let upcoming: Vec<_> = rows.iter().skip(1).map(queue_entry).collect();
        let now = std::time::Instant::now();
        self.state.apply(
            engine::Event::TrackChanged(engine::TrackInfo {
                uri: first.uri.clone(),
                name: first.name.clone(),
                artists: first.artists.clone(),
                duration_ms: first.duration_ms,
                cover_url: None,
            }),
            now,
        );
        self.state.apply(
            engine::Event::QueueChanged(engine::QueueState {
                context: None,
                index: 0,
                has_next: true,
                has_previous: false,
            }),
            now,
        );
        self.state.apply(
            engine::Event::Playing {
                position_ms: 41_000,
            },
            now,
        );
        self.now.queue = engine::QueueView {
            current: Some(queue_entry(&first)),
            context_remaining: upcoming.len(),
            upcoming,
            user_len: 0,
            context: None,
            shuffle: false,
            repeat: engine::RepeatMode::Off,
            radio: Some(RadioInfo {
                seed: "spotify:track:0000000000000000seed01".into(),
                title: "Gece Yolcusu".into(),
            }),
        };
        self.now.queue_known = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(uri: &str, context: Option<&str>) -> HiddenSong {
        HiddenSong {
            uri: uri.into(),
            context: context.map(Into::into),
            name: String::new(),
            artists: String::new(),
            hidden_at: 1,
        }
    }

    fn row(uri: &str) -> TrackRow {
        TrackRow {
            uri: uri.into(),
            name: uri.into(),
            artists: "A".into(),
            album: String::new(),
            duration_ms: 1,
            image: String::new(),
        }
    }

    #[test]
    fn menu_offers_hide_in_the_list_and_never_play_anywhere_for_visible_songs() {
        let none = HiddenList::default();
        let c = hide_choices(&none, Some("spotify:playlist:p"), &["t:a"]);
        assert_eq!(
            c,
            HideChoices {
                show: false,
                hide_here: true,
                hide_everywhere: true
            }
        );
        // Bağlamsız listede (çok dinlenenler, arama) yalnız "hiçbir yerde çalma".
        let c = hide_choices(&none, None, &["t:a"]);
        assert_eq!(
            c,
            HideChoices {
                show: false,
                hide_here: false,
                hide_everywhere: true
            }
        );
    }

    #[test]
    fn a_hidden_song_offers_show_and_a_list_hidden_one_still_offers_never_play() {
        let list = HiddenList::new(vec![song("t:a", Some("c")), song("t:b", None)]);
        // Bu listede gizli: Göster + (genel değil) Hiçbir yerde çalma.
        let c = hide_choices(&list, Some("c"), &["t:a"]);
        assert_eq!(
            c,
            HideChoices {
                show: true,
                hide_here: false,
                hide_everywhere: true
            }
        );
        // Genel gizli: Göster, başka giriş yok.
        let c = hide_choices(&list, Some("c"), &["t:b"]);
        assert_eq!(
            c,
            HideChoices {
                show: true,
                hide_here: false,
                hide_everywhere: false
            }
        );
        // Başka listede t:a gizli değil.
        assert!(!hide_choices(&list, Some("d"), &["t:a"]).show);
        // Karışık seçim (biri gizli, biri değil): gizlemeler sunulur, Göster değil.
        let c = hide_choices(&list, Some("c"), &["t:a", "t:z"]);
        assert_eq!(
            c,
            HideChoices {
                show: false,
                hide_here: true,
                hide_everywhere: true
            }
        );
        // Boş seçimde hiçbir şey "gizli" sayılmaz.
        assert!(!hide_choices(&list, Some("c"), &[]).show);
    }

    #[test]
    fn hiding_only_touches_songs_that_are_not_hidden_yet() {
        let list = HiddenList::new(vec![song("t:a", Some("c")), song("t:b", None)]);
        let rows = [row("t:a"), row("t:b"), row("t:z")];
        let here: Vec<String> = not_yet_hidden(&list, Some("c"), &rows)
            .into_iter()
            .map(|r| r.uri)
            .collect();
        assert_eq!(here, ["t:z"], "t:a bu listede, t:b her yerde gizli");
        let everywhere: Vec<String> = not_yet_hidden(&list, None, &rows)
            .into_iter()
            .map(|r| r.uri)
            .collect();
        assert_eq!(
            everywhere,
            ["t:a", "t:z"],
            "t:a yalnız bu listede gizli: genel gizlenebilir"
        );
    }

    #[test]
    fn the_context_key_comes_from_the_collection_or_the_album_page() {
        let playlist =
            ListKey::Collection(engine::CollectionId::Playlist("spotify:playlist:p".into()));
        assert_eq!(list_context(&playlist), Some("spotify:playlist:p"));
        assert_eq!(
            list_context(&ListKey::Collection(engine::CollectionId::Liked)),
            Some("liked")
        );
        assert_eq!(
            list_context(&ListKey::Page("album:spotify:album:a".into())),
            Some("spotify:album:a")
        );
        assert_eq!(
            list_context(&ListKey::Page("top:spotify:artist:x".into())),
            None
        );
        assert_eq!(list_context(&ListKey::Page("search:tarkan".into())), None);
    }

    #[test]
    fn a_private_session_never_writes_recently_played() {
        let mut recent = vec!["liked".to_string()];
        assert!(
            !note_recent(true, &mut recent, "spotify:album:a"),
            "gizli oturum yazmaz"
        );
        assert_eq!(recent, ["liked"]);
        assert!(note_recent(false, &mut recent, "spotify:album:a"));
        assert_eq!(recent, ["spotify:album:a", "liked"]);
        assert!(
            !note_recent(false, &mut recent, "spotify:album:a"),
            "başta zaten var: yazılmaz"
        );
        // Kapanınca yazım kaldığı yerden sürer.
        assert!(note_recent(false, &mut recent, "liked"));
        assert_eq!(recent, ["liked", "spotify:album:a"]);
    }

    #[test]
    fn the_radio_menu_entry_only_exists_for_valid_seeds() {
        assert!(Shell::radio_entry("spotify:track:abc123", "Ad").is_some());
        assert!(Shell::radio_entry("spotify:artist:abc123", "").is_some());
        assert!(Shell::radio_entry("liked", "Beğenilen Şarkılar").is_none());
        assert!(Shell::radio_entry("spotify:episode:abc", "").is_none());
        assert!(Shell::radio_entry("local:::x:1", "").is_none());
    }

    #[test]
    fn the_queue_head_names_the_radio_or_falls_back() {
        crate::i18n::with_lang(crate::i18n::Lang::En, || {
            let named = RadioInfo {
                seed: "s".into(),
                title: "Gece Yolcusu".into(),
            };
            assert_eq!(Shell::radio_head(&named), "Next from radio: Gece Yolcusu");
            let plain = RadioInfo {
                seed: "s".into(),
                title: "  ".into(),
            };
            assert_eq!(Shell::radio_head(&plain), "Next from radio");
        });
        crate::i18n::with_lang(crate::i18n::Lang::Tr, || {
            let named = RadioInfo {
                seed: "s".into(),
                title: "Gece Yolcusu".into(),
            };
            assert_eq!(Shell::radio_head(&named), "Sıradaki: Gece Yolcusu radyosu");
        });
    }
}
