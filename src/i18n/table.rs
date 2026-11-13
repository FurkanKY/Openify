//! Arayüz yazılarının tablosu: HER girdi İKİ dili birden ister (`keys!` makrosu,
//! bkz. `src/i18n.rs`). Yeni yazı eklemek için: anahtar adını seç (ekran + anlam,
//! `SettingsLanguageTitle` gibi), `en` ve `tr`'yi yaz, kodda `t!(Anahtar)` kullan.
//!
//! Üslup: İngilizce kısa, doğal, sakin ("Apple Music" / "Spotify" tonu), birebir
//! çeviri değil; Türkçe mevcut metinler aynen. Çoğul: `"{} song|{} songs"`.
//! Görünen ad için `app_name!()` / `app_name_acc!()` (bkz. `src/brand.rs`).

#[allow(unused_imports)]
use crate::brand::{app_name, app_name_acc, app_name_dat};

keys! {
    // ── Dil ayarı ──
    SettingsLanguageTitle { en: "Dil / Language", tr: "Dil / Language" }
    SettingsLanguageDesc { en: "The language of the interface. “System” follows your Windows display language.", tr: "Arayüzün dili. “Sistem”, Windows'un görüntü dilini izler." }
    LangSystem { en: "System", tr: "Sistem" }
    LangEnglish { en: "English", tr: "English" }
    LangTurkish { en: "Türkçe", tr: "Türkçe" }

    // ── src/ui/explore.rs ──
    NavHome { en: "Home", tr: "Ana sayfa" }
    NavExplore { en: "Explore", tr: "Keşfet" }
    NavLibrary { en: "Library", tr: "Kütüphane" }
    ExploreMeta { en: "New releases, trending playlists, genres and moods", tr: "Yeni çıkanlar, trend listeler, türler ve ruh hâlleri" }
    ExploreNew { en: "New releases", tr: "Yeni çıkanlar" }
    ExploreCharts { en: "Trending playlists", tr: "Trend listeler" }
    ExploreGenres { en: "Genres and moods", tr: "Tür ve ruh hâli" }
    ExploreSimilar { en: "Similar artists you might like", tr: "Benzer sanatçılardan öneriler" }

    // ── src/ui/components.rs ──
    ErrForbiddenTitle { en: "No access to this playlist", tr: "Bu listeye erişim yok" }
    ErrForbiddenBody { en: "Spotify isn't sharing this playlist, or it has been removed.", tr: "Spotify bu listeyi paylaşmıyor ya da liste kaldırılmış." }
    ErrOfflineTitle { en: "Can't reach Spotify", tr: "Spotify'a ulaşılamadı" }
    ErrOfflineBody { en: "Check your internet connection, then try again.", tr: "İnternet bağlantını kontrol et. Bağlantı gelince yeniden dene." }
    ErrBusyTitle { en: "Spotify is busy right now", tr: "Spotify şu an yoğun" }
    ErrBusyBody { en: "Wait a moment and try again.", tr: "Biraz bekleyip yeniden dene." }
    ErrFailedTitle { en: "Couldn't load", tr: "Alınamadı" }
    ErrFailedBody { en: "Spotify sent an unexpected response. Trying again usually fixes it.", tr: "Spotify beklenmedik bir yanıt verdi. Yeniden denemek çoğu zaman yeter." }

    // ── src/ui/home.rs ──
    HomeGreetMorning { en: "Good morning", tr: "Günaydın" }
    HomeGreetAfternoon { en: "Good afternoon", tr: "İyi günler" }
    HomeGreetEvening { en: "Good evening", tr: "İyi akşamlar" }
    HomeGreetNight { en: "Good night", tr: "İyi geceler" }
    HomeTagMorning { en: "Start the day with a great song.", tr: "Güne iyi bir şarkıyla başla." }
    HomeTagAfternoon { en: "Pick up where you left off, or try something new.", tr: "Kaldığın yerden devam et ya da yeni bir şey aç." }
    HomeTagEvening { en: "The day is done. Let the music begin.", tr: "Gün bitti, müzik başlasın." }
    HomeTagNight { en: "For a quiet night.", tr: "Sakin bir gece için." }
    HomeOftenTitle { en: "Your most played", tr: "Çok dinlediklerin" }
    HomeOftenDesc { en: "Playlists and albums you've opened the most lately.", tr: "Son zamanlarda en çok açtığın listeler ve albümler." }
    HomeListsTitle { en: "Your playlists", tr: "Listelerin" }
    HomeListsDesc { en: "Everything you've made and saved lives in your Library.", tr: "Oluşturduklarının ve kaydettiklerinin hepsi Kütüphane'de." }
    HomeNewTitle { en: "New for you", tr: "Senin için yeni" }
    HomeNewDescMade { en: "Made for you by Spotify.", tr: "Spotify'ın senin için hazırladıkları." }
    HomeNewDescWeekly { en: "Your weekly discoveries and daily mixes.", tr: "Haftalık keşiflerin ve günlük karışımların." }
    HomeDailyMixes { en: "Daily Mixes", tr: "Günlük karışımlar" }
    HomeMixCount { en: "{} mix|{} mixes", tr: "{} karışım" }
    Pause { en: "Pause", tr: "Duraklat" }
    PlayAgain { en: "Play again", tr: "Baştan çal" }
    Resume { en: "Resume", tr: "Devam et" }
    FullScreen { en: "Full screen", tr: "Tam ekran" }
    NowPlaying { en: "Now playing", tr: "Şimdi çalıyor" }
    NowTab { en: "Playing", tr: "Çalıyor" }
    LikedSongs { en: "Liked Songs", tr: "Beğenilenler" }
    Songs { en: "{} song|{} songs", tr: "{} şarkı" }
    RecentlyPlayed { en: "Recently played", tr: "Son çalınan" }
    Play { en: "Play", tr: "Çal" }

    // ── src/ui/library.rs ──
    LibraryYours { en: "Your Library", tr: "Kütüphanen" }
    Album { en: "Album", tr: "Albüm" }
    LibraryFollowedList { en: "Followed playlist", tr: "Takip edilen liste" }
    LibraryYourPlaylist { en: "Your playlist", tr: "Senin listen" }
    Loading { en: "Loading…", tr: "Yükleniyor…" }
    BackToLiked { en: "Back to Liked Songs", tr: "Beğenilenlere dön" }
    Retry { en: "Try again", tr: "Yeniden dene" }
    LibraryLoadFailed { en: "couldn't load", tr: "alınamadı" }
    EmptyListTitle { en: "This playlist is empty", tr: "Bu liste boş" }
    EmptyListBody { en: "Songs you add to this playlist in Spotify will show up here.", tr: "Spotify'da bu listeye şarkı eklediğinde burada görünür." }
    NoMatchTitle { en: "No matching songs", tr: "Eşleşen şarkı yok" }
    NoMatchBody { en: "Try another word, or press Esc to close search.", tr: "Başka bir kelime dene ya da Esc ile bulmayı kapat." }
    UnlikeTip { en: "Remove from Liked Songs", tr: "Beğenilenlerden çıkar" }
    LikeTip { en: "Add to Liked Songs", tr: "Beğen" }
    RemoveFromLibrary { en: "Remove from Your Library", tr: "Kütüphanenden çıkar" }
    MoreTip { en: "More", tr: "Diğer" }
    FindInList { en: "Find in this playlist", tr: "Bu listede bul" }
    CloseFind { en: "Close search", tr: "Bulmayı kapat" }
    ColTitle { en: "Title", tr: "Şarkı" }
    ColDuration { en: "Duration", tr: "Süre" }
    DurMinutes { en: "{} min", tr: "{} dk" }
    DurHoursMinutes { en: "{} hr {} min", tr: "{} sa {} dk" }

    // ── src/ui/lyrics_view.rs ──
    NothingPlaying { en: "Nothing playing", tr: "Hiçbir şey çalmıyor" }
    LyricsIdleBody { en: "When a song plays, its lyrics scroll here in time.", tr: "Bir şarkı çalınca sözleri burada, şarkıyla birlikte akar." }
    LyricsNoneTitle { en: "No lyrics for this song", tr: "Bu şarkının sözü yok" }
    LyricsNoneBody { en: "Not found on Spotify or LRCLIB.", tr: "Ne Spotify'da ne LRCLIB'de bulundu." }
    LyricsLoading { en: "Fetching lyrics…", tr: "Sözler getiriliyor…" }
    LyricsUnsynced { en: "These lyrics aren't synced to the song", tr: "Bu sözler şarkıyla eşzamanlı değil" }
    LyricsProvider { en: "Lyrics: {}", tr: "Sözler: {}" }

    // ── src/ui/fullscreen.rs ──
    ShuffleOff { en: "Turn off shuffle", tr: "Karıştırmayı kapat" }
    ShuffleOn { en: "Shuffle", tr: "Karıştır" }
    PrevTrack { en: "Previous", tr: "Önceki şarkı" }
    KeySpace { en: "Space", tr: "Boşluk" }
    NextTrack { en: "Next", tr: "Sonraki şarkı" }
    HideLyrics { en: "Hide lyrics", tr: "Sözleri gizle" }
    ShowLyrics { en: "Show lyrics", tr: "Sözleri göster" }
    ExitFullScreen { en: "Exit full screen", tr: "Tam ekrandan çık" }

    // ── src/ui/shelf.rs ──
    KindLists { en: "Playlists", tr: "Listeler" }
    KindAlbums { en: "Albums", tr: "Albümler" }
    SortName { en: "Alphabetical", tr: "Alfabetik" }
    LibraryLoadError { en: "Couldn't load your library: {}. {}", tr: "Kütüphane alınamadı: {}. {}" }
    LikedSearchWords { en: "Liked Songs liked songs favorites", tr: "Beğenilenler beğenilen şarkılar" }
    FindInLibrary { en: "Search your library", tr: "Kütüphanede ara" }
    SortTip { en: "Sort: {}", tr: "Sıralama: {}" }
    ViewGrid { en: "Grid", tr: "Izgara" }
    ViewList { en: "List", tr: "Liste" }
    NewPlaylist { en: "New playlist", tr: "Yeni liste" }
    LibraryEmptyTitle { en: "Your library is empty", tr: "Kütüphanen henüz boş" }
    LibraryEmptyBody { en: "Add a playlist or album from Explore and it will show up here.", tr: "Keşfet'ten bir liste ya da albüm ekleyince burada görünür." }
    NoMatchingPlaylists { en: "No matching playlists", tr: "Eşleşen liste yok" }
    NoMatchingPlaylistsBody { en: "Try another name, or clear the filter.", tr: "Başka bir ad dene ya da süzgeci kaldır." }
    KindForYou { en: "Made for you", tr: "Sana özel" }
    Playlist { en: "Playlist", tr: "Liste" }

    // ── src/ui/now_panel.rs ──
    NowEmptyBody { en: "Play a playlist. The current song, what's next and the lyrics appear here.", tr: "Bir liste çal; çalan şarkı, sıradaki ve sözler burada görünür." }
    NowUpNext { en: "Up next", tr: "Sıradaki" }
    Lyrics { en: "Lyrics", tr: "Sözler" }
    NowArtistPage { en: "Artist page", tr: "Sanatçı sayfası" }
    Artist { en: "Artist", tr: "Sanatçı" }
    NowRecentHint { en: "Pick something you played recently to start it over.", tr: "Son çalınanlardan birini seç, baştan çalsın." }
    NowRecentTitle { en: "Recently played", tr: "Son çalınanlar" }

    // ── src/ui/popups.rs ──
    TrayOpen { en: concat!("Open ", app_name!()), tr: concat!(app_name_acc!(), " aç") }
    TrayQuit { en: "Quit", tr: "Çık" }

    // ── src/ui/pill.rs ──
    PillNoAudioTitle { en: "No audio output", tr: "Ses çıkışı yok" }
    PillNoAudioDetail { en: "Connect speakers or headphones", tr: "Hoparlör ya da kulaklık bağla" }
    Offline { en: "Offline", tr: "Çevrimdışı" }
    OfflineLibrary { en: "Showing your saved library", tr: "Kütüphane önbellekten" }
    OfflineWaiting { en: "Waiting for a connection", tr: "Bağlantı bekleniyor" }
    PlayNamed { en: "Play {}", tr: "Çal: {}" }
    Unmute { en: "Unmute", tr: "Sesi aç" }
    Mute { en: "Mute", tr: "Sessize al" }
    QueueTip { en: "Queue", tr: "Sıradaki" }
    Close { en: "Close", tr: "Kapat" }
    OfflineFull { en: "Offline · Showing your saved library", tr: "Çevrimdışı · Kütüphane önbellekten" }
    RepeatTipOff { en: "Repeat", tr: "Tekrarla" }
    RepeatTipAll { en: "Repeat this song", tr: "Bu şarkıyı tekrarla" }
    RepeatTipOne { en: "Turn off repeat", tr: "Tekrarı kapat" }

    // ── src/ui/queue_panel.rs ──
    QueueTab { en: "Queue", tr: "Sıradaki" }
    QueueEmptyTitle { en: "Nothing in the queue", tr: "Sırada şarkı yok" }
    QueueEmptyBody { en: "Play a playlist or add songs to the queue. What's coming up shows here.", tr: "Bir liste çal ya da şarkıları sıraya ekle; çalınacaklar burada görünür." }
    QueueUserHead { en: "Next in queue", tr: "Sıradaki" }
    QueueNextFrom { en: "Next from: {}", tr: "Sıradaki: {}" }
    QueueNextFromThis { en: "Next from this list", tr: "Sıradaki: bu listeden" }
    QueueShuffled { en: "Shuffled", tr: "Karışık" }
    Clear { en: "Clear", tr: "Temizle" }
    QueueInfoLoading { en: "Loading info…", tr: "Bilgisi geliyor…" }
    QueueRemoveTip { en: "Remove from queue", tr: "Sıradan çıkar" }

    // ── src/ui/pages.rs ──
    ReleaseSingle { en: "Single", tr: "Tekli" }
    ReleaseCompilation { en: "Compilation", tr: "Derleme" }
    Back { en: "Back", tr: "Geri" }
    Forward { en: "Forward", tr: "İleri" }
    ShowLess { en: "Show less", tr: "Daha az" }
    ShowAll { en: "Show all", tr: "Tümünü göster" }
    ArtistAlbumCount { en: "{} album|{} albums", tr: "{} albüm" }
    ArtistSingleCount { en: "{} single & EP|{} singles & EPs", tr: "{} tekli ve EP" }
    PagePopular { en: "Popular", tr: "Popüler" }
    PageSingles { en: "Singles and EPs", tr: "Tekliler ve EP'ler" }
    PageCompilations { en: "Compilations", tr: "Derlemeler" }
    PageAppearsOn { en: "Appears on", tr: "Şunlarda da var" }
    PageRelated { en: "Similar artists", tr: "Benzer sanatçılar" }
    AddToLibrary { en: "Add to Your Library", tr: "Kütüphanene ekle" }
    PageMoreFrom { en: "More from {}", tr: "{} sanatçısından daha fazlası" }
    SearchSearching { en: "Searching Spotify…", tr: "Spotify'da aranıyor…" }
    SearchNoResults { en: "No results on Spotify. Try something else.", tr: "Spotify'da sonuç yok. Başka bir şey dene." }
    SearchResults { en: "{} result on Spotify|{} results on Spotify", tr: "Spotify'da {} sonuç" }
    SectionSongs { en: "Songs", tr: "Şarkılar" }
    SectionArtists { en: "Artists", tr: "Sanatçılar" }

    // ── src/ui/palette.rs ──
    CmdTogglePlay { en: "Play / Pause", tr: "Çal / Duraklat" }
    CmdNext { en: "Next song", tr: "Sonraki şarkı" }
    CmdPrevious { en: "Previous song", tr: "Önceki şarkı" }
    CmdToggleMute { en: "Mute / Unmute", tr: "Sessize al / Sesi aç" }
    CmdRefreshLibrary { en: "Refresh library", tr: "Kütüphaneyi yenile" }
    CmdRefreshLibraryDesc { en: "Update from Spotify now", tr: "Spotify'dan hemen tazele" }
    CmdHomeDesc { en: "Recently played and picks made for you", tr: "Son çalınanlar ve senin için hazırlananlar" }
    CmdExploreDesc { en: "New releases, trending playlists, genres", tr: "Yeni çıkanlar, trend listeler, türler" }
    CmdLibraryDesc { en: "Your playlists and albums", tr: "Listelerin ve albümlerin" }
    CmdNewPlaylistDesc { en: "Create an empty playlist", tr: "Boş bir liste oluştur" }
    Settings { en: "Settings", tr: "Ayarlar" }
    CmdSettingsDesc { en: "Sound, appearance, startup, storage", tr: "Ses, görünüm, başlangıç, depolama" }
    Shortcuts { en: "Keyboard shortcuts", tr: "Kısayollar" }
    CmdShortcutsDesc { en: "Every key", tr: "Bütün tuşlar" }
    SignOut { en: "Sign out", tr: "Çıkış yap" }
    CmdSignOutDesc { en: concat!("Sign out of Spotify and quit ", app_name!()), tr: concat!("Spotify hesabından çıkar ve ", app_name_acc!(), " kapatır") }
    ShPlayPause { en: "Play / pause", tr: "Çal / duraklat" }
    ShSeekFwd { en: "Skip forward 5 s", tr: "5 sn ileri sar" }
    ShSeekBack { en: "Skip back 5 s", tr: "5 sn geri sar" }
    ShVolUp { en: "Volume up", tr: "Sesi artır" }
    ShVolDown { en: "Volume down", tr: "Sesi azalt" }
    ShMute { en: "Mute / unmute", tr: "Sessize al / sesi aç" }
    ShShuffle { en: "Turn shuffle on / off", tr: "Karıştır aç / kapat" }
    ShRepeat { en: "Repeat: off, all, one", tr: "Tekrar: kapalı, hepsi, tek" }
    ShQueuePanel { en: "Queue panel", tr: "Sıradaki şarkılar paneli" }
    ShLyricsPanel { en: "Lyrics panel", tr: "Sözler paneli" }
    ShFullScreen { en: "Full screen (Esc to exit)", tr: "Tam ekran (Esc çıkar)" }
    ShMoveInQueue { en: "Move the selected song in the queue", tr: "Sırada seçili şarkıyı taşı" }
    ShSearch { en: "Search, run a command", tr: "Ara, komut çalıştır" }
    KeyCtrlShiftSpace { en: "Ctrl Shift Space", tr: "Ctrl Shift Boşluk" }
    ShGlobalSearch { en: "Search from any app", tr: "Başka uygulamadayken de ara" }
    ShPasteLink { en: "Play the Spotify link on the clipboard", tr: "Panodaki Spotify bağlantısını çal" }
    ShSelectSong { en: "Select a song in a list or the queue", tr: "Listede ya da sırada şarkı seç" }
    ShEnter { en: "Play selected song; open shelf card (Library: first result)", tr: "Seçili şarkıyı çal; rafta kartı aç (Kütüphane: ilk sonucu)" }
    ShTab { en: "Moves focus: search, shelf, list, player bar", tr: "Sıra: arama, raf, liste, hap" }
    ShArrows { en: "Pick a card on the shelf or strip", tr: "Rafta ya da şeritte kart seç" }
    ShHomeEnd { en: "First / last card on the shelf", tr: "Rafta ilk / son kart" }
    ShTypeFilter { en: "Type to filter, Esc clears the text", tr: "Yazarak süz, Esc yazıyı siler" }
    ShEsc { en: "Close the palette, release focus", tr: "Paleti kapat, odağı bırak" }
    ShBackForward { en: "Back / forward (mouse side buttons too)", tr: "Geri / ileri (farenin yan tuşları da)" }
    ShTabHome { en: "Home tab", tr: "Ana sayfa sekmesi" }
    ShTabExplore { en: "Explore tab", tr: "Keşfet sekmesi" }
    ShTabLibrary { en: "Library tab", tr: "Kütüphane sekmesi" }
    ShFind { en: "Find in the open playlist (Esc closes)", tr: "Açık listede bul (Esc kapatır)" }
    ShSelectAll { en: "Select every song in the list", tr: "Listedeki bütün şarkıları seç" }
    KeyCtrlClick { en: "Ctrl click", tr: "Ctrl tık" }
    ShMultiSelect { en: "Add to / remove from the selection (Shift: range)", tr: "Seçime ekle / çıkar (Shift: aralık)" }
    ShUndo { en: "Undo the last change", tr: "Son değişikliği geri al" }
    ShDelete { en: "Remove the selected songs (from your own playlist or the queue)", tr: "Seçili şarkıları çıkar (kendi listenden ya da sıradan)" }
    ShRename { en: "Rename the open playlist", tr: "Açık listeni yeniden adlandır" }
    ShContextMenu { en: "Menu for the selected song (like right-click)", tr: "Seçili şarkının menüsü (sağ tık gibi)" }
    SearchPlaceholder { en: "What are you in the mood for?", tr: "Bugün ne dinlemek istersin?" }
    PaletteListenAgain { en: "Listen again", tr: "Tekrar dinle" }
    PaletteBrowseGenres { en: "Browse by genre", tr: "Türlere göz at" }
    PaletteInvite { en: "Search for artists, songs, albums and playlists.", tr: "Sanatçı, şarkı, albüm ve liste ara." }
    PaletteTopResult { en: "Top result", tr: "En iyi sonuç" }
    PaletteAlbumsLists { en: "Albums and playlists", tr: "Albümler ve listeler" }
    PaletteNoResultsFor { en: "Nothing found for “{}”", tr: "“{}” için bir şey bulunamadı" }
    PaletteActions { en: "Actions", tr: "Eylemler" }
    PaletteAllResultsFor { en: "All results for “{}”", tr: "“{}” için tüm sonuçlar" }
    KindSong { en: "Song", tr: "Şarkı" }
    PaletteRecentSearches { en: "Recent searches", tr: "Son aramalar" }
    PaletteConfirmSignOut { en: "Click again or press Enter to sign out", tr: "Çıkış yapmak için yeniden tıkla ya da Enter'a bas" }
    PaletteConfirmSignOutDesc { en: concat!("Your saved sign-in is deleted and ", app_name!(), " quits · Esc to cancel"), tr: concat!("Kayıtlı giriş silinir, ", app_name!(), " kapanır · Vazgeçmek için Esc") }
    PaletteSearchOffline { en: "Can't reach Spotify. Searching your library only.", tr: "Spotify'a ulaşılamadı; yalnızca kütüphanende aranıyor." }
    PaletteSearchBusy { en: "Spotify is busy right now. Searching your library only.", tr: "Spotify şu an yoğun; yalnızca kütüphanende aranıyor." }
    PaletteSearchFailed { en: "Couldn't search Spotify. Searching your library only.", tr: "Spotify'da aranamadı; yalnızca kütüphanende aranıyor." }

    // ── src/ui/manage.rs ──
    FailLikeDenied { en: "Spotify didn't allow this change to your likes.", tr: "Spotify bu beğeni değişikliğine izin vermedi." }
    FailAlbumDenied { en: "Spotify didn't allow this album change.", tr: "Spotify albüm değişikliğine izin vermedi." }
    FailListContentNotYours { en: "This playlist isn't yours, and Spotify didn't allow the change.", tr: "Bu liste sana ait değil, Spotify değişikliğe izin vermedi." }
    FailListDenied { en: "Spotify didn't allow this playlist change.", tr: "Spotify bu liste değişikliğine izin vermedi." }
    FailRenameNotYours { en: "This playlist isn't yours, so you can't rename it.", tr: "Bu liste sana ait değil, adını değiştiremezsin." }
    FailRenameDenied { en: "Spotify didn't allow renaming this playlist.", tr: "Spotify listenin adını değiştirmene izin vermedi." }
    FailCreateDenied { en: "Spotify didn't allow creating a new playlist.", tr: "Spotify yeni liste oluşturmana izin vermedi." }
    FailRemoveDenied { en: "Spotify didn't allow this.", tr: "Spotify bu işleme izin vermedi." }
    FailGoneSong { en: "This song is no longer on Spotify.", tr: "Şarkı Spotify'da artık yok." }
    FailGoneAlbum { en: "This album is no longer on Spotify.", tr: "Albüm Spotify'da artık yok." }
    FailGoneListOrSong { en: "This playlist or song is no longer on Spotify.", tr: "Liste ya da şarkı Spotify'da artık yok." }
    FailGoneList { en: "This playlist is no longer on Spotify.", tr: "Liste Spotify'da artık yok." }
    FailGoneCreate { en: "A song you tried to add to the playlist is no longer on Spotify.", tr: "Listeye eklenmek istenen bir şarkı Spotify'da artık yok." }
    FailOffline { en: "Can't reach Spotify. Check your connection and try again.", tr: "Spotify'a ulaşılamadı. Bağlantını kontrol edip yeniden dene." }
    FailExpired { en: "Spotify has been unreachable for days, so the change couldn't be saved.", tr: "Günlerdir Spotify'a ulaşılamadı, değişiklik yazılamadı." }
    FailUnauthorized { en: concat!("Your Spotify session is no longer valid. Reopen ", app_name!(), "."), tr: concat!("Spotify oturumun geçersiz. ", app_name_acc!(), " yeniden aç.") }
    FailConflict { en: "The playlist was changed somewhere else too. Try again.", tr: "Liste başka bir yerde de değişti. Yeniden dene." }
    FailServer { en: "Spotify is having trouble right now. Try again in a moment.", tr: "Spotify şu an hata veriyor. Biraz sonra yeniden dene." }
    FailRateLimited { en: "Spotify says there were too many requests. Try again in a moment.", tr: "Spotify çok fazla istek geldiğini söyledi. Biraz sonra yeniden dene." }
    FailUnsupported { en: "Spotify doesn't support this.", tr: "Spotify bu işlemi desteklemiyor." }
    FailInvalid { en: "This change couldn't be sent to Spotify (invalid request).", tr: "Bu değişiklik Spotify'a gönderilemedi (geçersiz istek)." }
    FailParent { en: "The playlist had to be created first, and that didn't work.", tr: "Önce listenin oluşturulması gerekiyordu, o yapılamadı." }
    FailUnknown { en: "Spotify didn't accept this change.", tr: "Spotify bu değişikliği kabul etmedi." }
    ToastLiked { en: "Added to Liked Songs", tr: "Beğenilenlere eklendi" }
    ToastUnliked { en: "Removed from Liked Songs", tr: "Beğenilenlerden çıkarıldı" }
    ToastLikedN { en: "{} songs added to Liked Songs", tr: "{} şarkı beğenilenlere eklendi" }
    ToastUnlikedN { en: "{} songs removed from Liked Songs", tr: "{} şarkı beğenilenlerden çıkarıldı" }
    ToastAlbumAdded { en: "Album added to Your Library", tr: "Albüm kütüphanene eklendi" }
    ToastAlbumRemoved { en: "Album removed from Your Library", tr: "Albüm kütüphanenden çıkarıldı" }
    ToastNothingToUndo { en: "Nothing to undo", tr: "Geri alınacak bir şey yok" }
    ToastUndone { en: "Undone: {}", tr: "Geri alındı: {}" }
    ToastDropped { en: "Change dropped. Back to how it is on Spotify", tr: "Değişiklik bırakıldı, Spotify'daki hâline döndü" }
    ToastMoved1 { en: "Song moved", tr: "Şarkı taşındı" }
    ToastMovedN { en: "{} songs moved", tr: "{} şarkı taşındı" }
    ToastAddedToList { en: "Added to “{}”", tr: "“{}” listesine eklendi" }
    ToastAddedToListN { en: "{} songs added to “{}”", tr: "{} şarkı “{}” listesine eklendi" }
    ToastRemovedFromList { en: "Removed from playlist", tr: "Listeden çıkarıldı" }
    ToastRemovedFromListN { en: "{} songs removed from playlist", tr: "{} şarkı listeden çıkarıldı" }
    ToastCreated { en: "“{}” created", tr: "“{}” oluşturuldu" }
    ToastCreatedWith1 { en: "“{}” created with a song", tr: "“{}” oluşturuldu, şarkı eklendi" }
    ToastCreatedWithN { en: "“{}” created with {} songs", tr: "“{}” oluşturuldu, {} şarkı eklendi" }
    ToastRenamed { en: "Playlist renamed", tr: "Liste yeniden adlandırıldı" }
    ToastListDeleted { en: "Playlist deleted", tr: "Liste silindi" }
    ToastListRemoved { en: "Playlist removed from Your Library", tr: "Liste kütüphanenden kaldırıldı" }
    MenuAddToQueue { en: "Add to queue", tr: "Sıraya ekle" }
    MenuAddToQueueN { en: "Add {} songs to queue", tr: "{} şarkıyı sıraya ekle" }
    MenuPlayNext { en: "Play next", tr: "Sonra çal" }
    MenuPlayNextN { en: "Play {} songs next", tr: "{} şarkıyı sonra çal" }
    MenuNewPlaylistDots { en: "New playlist…", tr: "Yeni liste…" }
    MenuAddToPlaylist { en: "Add to playlist", tr: "Listeye ekle" }
    MenuGoAlbum { en: "Go to album", tr: "Albüme git" }
    MenuGoArtist { en: "Go to artist", tr: "Sanatçıya git" }
    MenuCopyLink { en: "Copy link", tr: "Bağlantıyı kopyala" }
    MenuCopyLinkN { en: "Copy {} links", tr: "{} bağlantıyı kopyala" }
    MenuRemoveFromList { en: "Remove from playlist", tr: "Listeden çıkar" }
    MenuRemoveFromListN { en: "Remove {} songs from playlist", tr: "{} şarkıyı listeden çıkar" }
    MenuOpen { en: "Open", tr: "Aç" }
    MenuRename { en: "Rename", tr: "Yeniden adlandır" }
    MenuDeleteList { en: "Delete playlist", tr: "Listeyi sil" }
    MenuRemoveFromLibrary { en: "Remove from Your Library", tr: "Kütüphanenden kaldır" }
    ToastQueued { en: "Added to queue", tr: "Sıraya eklendi" }
    ToastQueuedN { en: "{} songs added to queue", tr: "{} şarkı sıraya eklendi" }
    ToastPlayNext { en: "Will play next", tr: "Sonra çalacak" }
    ToastPlayNextN { en: "{} songs will play next", tr: "{} şarkı sonra çalacak" }
    ToastLinkCopied { en: "Link copied", tr: "Bağlantı kopyalandı" }
    ToastLinksCopied { en: "{} links copied", tr: "{} bağlantı kopyalandı" }
    Undo { en: "Undo", tr: "Geri al" }
    PromptCreate { en: "Create", tr: "Oluştur" }
    PromptNewPlaylistSongs { en: "New playlist · {} song|New playlist · {} songs", tr: "Yeni liste · {} şarkı" }
    PromptRenameTitle { en: "Rename playlist", tr: "Listeyi yeniden adlandır" }
    Save { en: "Save", tr: "Kaydet" }
    PromptNameHint { en: "Playlist name", tr: "Listenin adı" }
    Cancel { en: "Cancel", tr: "Vazgeç" }
    SyncFailed { en: "{} change couldn't be saved to Spotify|{} changes couldn't be saved to Spotify", tr: "{} değişiklik Spotify'a yazılamadı" }
    SyncWriting { en: "Saving {} change to Spotify|Saving {} changes to Spotify", tr: "Spotify'a yazılıyor: {} değişiklik" }
    SyncTitle { en: "Sync", tr: "Eşitleme" }
    SyncWaiting { en: "{} change waiting. It will be saved to Spotify once you're back online.|{} changes waiting. They will be saved to Spotify once you're back online.", tr: "{} değişiklik bekliyor: bağlantı gelince Spotify'a yazılacak." }
    SyncSaving { en: "Saving {} change to Spotify…|Saving {} changes to Spotify…", tr: "{} değişiklik Spotify'a yazılıyor…" }
    SyncStillShown { en: "It still looks changed here. If you drop it, it goes back to how it is on Spotify.", tr: "Ekranda hâlâ değişmiş görünüyor. Bırakırsan Spotify'daki hâline döner." }
    SyncDrop { en: "Drop", tr: "Bırak" }
    ASong { en: "A song", tr: "Bir şarkı" }
    AListFallback { en: "a playlist", tr: "bir liste" }
    FailedAlbumAdd { en: "Couldn't add the album to your library", tr: "Albüm kütüphaneye eklenemedi" }
    FailedAlbumRemove { en: "Couldn't remove the album from your library", tr: "Albüm kütüphaneden çıkarılamadı" }
    FailedLike { en: "Couldn't like {}", tr: "{} beğenilemedi" }
    FailedUnlike { en: "Couldn't remove {} from Liked Songs", tr: "{} beğenilerden çıkarılamadı" }
    FailedAddToList { en: "Couldn't add {} song to {}|Couldn't add {} songs to {}", tr: "{} şarkı {} listesine eklenemedi" }
    FailedRemoveFromList { en: "Couldn't remove {} song from {}|Couldn't remove {} songs from {}", tr: "{} şarkı {} listesinden çıkarılamadı" }
    FailedCreate { en: "Couldn't create “{}”", tr: "“{}” listesi oluşturulamadı" }
    FailedRename { en: "Couldn't rename the playlist to “{}”", tr: "Liste “{}” olarak adlandırılamadı" }
    FailedDelete { en: "Couldn't delete {}", tr: "{} silinemedi" }
    FailedRestore { en: "Couldn't restore {}", tr: "{} geri getirilemedi" }
    FailedReorder { en: "Couldn't reorder the songs in {}", tr: "{} listesinde sıra değiştirilemedi" }

    // ── src/ui/settings.rs ──
    UpdNotInstalled { en: "This copy isn't an installed version, so there's no update check.", tr: "Bu kopya kurulu sürüm değil, güncelleme denetimi yok." }
    UpdOff { en: "Update checks are off.", tr: "Güncelleme denetimi kapalı." }
    UpdWaiting { en: "Updates are checked in the background.", tr: "Güncellemeler arka planda denetlenir." }
    UpdChecking { en: "Checking…", tr: "Denetleniyor…" }
    UpdUpToDate { en: concat!(app_name!(), " is up to date."), tr: concat!(app_name!(), " güncel.") }
    UpdReady { en: concat!("Version {} is ready. It applies when you reopen ", app_name!(), "."), tr: concat!("{} sürümü hazır. ", app_name_acc!(), " yeniden açınca uygulanır.") }
    UpdRolledBack { en: "Went back to the previous version. {} didn't start properly.", tr: "Önceki sürüme dönüldü. {} sürümü açılmadı." }
    UpdFailed { en: "Couldn't check for updates. Check your internet connection.", tr: "Denetlenemedi. İnternet bağlantını kontrol et." }
    SystemDefault { en: "System default", tr: "Sistem varsayılanı" }
    DeviceDisconnected { en: "{} (not connected)", tr: "{} (bağlı değil)" }
    LicLibrespot { en: "Spotify connection and audio decoding", tr: "Spotify bağlantısı ve ses çözme" }
    LicUi { en: "Interface", tr: "Arayüz" }
    LicInstaller { en: "Installation and updates", tr: "Kurulum ve güncelleme" }
    LicAudioOut { en: "Audio output", tr: "Ses çıkışı" }
    LicSqliteTerms { en: "MIT, SQLite public domain", tr: "MIT, SQLite kamu malı" }
    Cache { en: "Cache", tr: "Önbellek" }
    LicNetwork { en: "Networking", tr: "Ağ" }
    LicDataFormats { en: "Data formats", tr: "Veri biçimleri" }
    LicImages { en: "Icon and image handling", tr: "Simge ve resim işleme" }
    LicMatcher { en: "Search matching", tr: "Aramada eşleştirme" }
    LicWindows { en: "Windows integration", tr: "Windows ile bütünleşme" }
    LicTitleFont { en: "Heading typeface", tr: "Başlık yazı tipi" }
    LicFallbackFont { en: "Fallback typeface", tr: "Yedek yazı tipi" }
    LicIcons { en: "Icons", tr: "Simgeler" }
    SignOutFailed { en: concat!("Couldn't sign out. Close ", app_name!(), " and try again."), tr: concat!("Çıkış yapılamadı. ", app_name_acc!(), " kapatıp yeniden dene.") }
    CacheCleared { en: "Cleared: {} freed.", tr: "Temizlendi: {} boşaldı." }
    YourSpotifyAccount { en: "Your Spotify account", tr: "Spotify hesabın" }
    Account { en: "Account", tr: "Hesap" }
    ProfileTip { en: "Profile and settings", tr: "Profil ve ayarlar" }
    SpotifyAccount { en: "Spotify account", tr: "Spotify hesabı" }
    ShownWhenConnected { en: "Appears once connected", tr: "Bağlanınca görünür" }
    About { en: "About", tr: "Hakkında" }
    ConfirmSignOutTitle { en: "Sign out?", tr: "Çıkış yapılsın mı?" }
    ConfirmSignOutBody { en: concat!("You'll be signed out of Spotify and ", app_name!(), " will quit. To keep using it, you'll need to sign in again."), tr: concat!("Spotify hesabından çıkılır ve ", app_name!(), " kapanır. Kullanmaya devam etmek için yeniden giriş yapman gerekir.") }
    ConfirmClearTitle { en: "Clear the cache?", tr: "Önbellek temizlensin mi?" }
    ConfirmClearBody { en: "Cover art and downloaded song files are deleted{}. They download again when needed. Your playlists, settings and sign-in aren't affected.", tr: "Kapaklar ve indirilmiş şarkı dosyaları silinir{}. İhtiyaç olunca yeniden iner. Listelerin, ayarların ve girişin etkilenmez." }
    QualityHigh { en: "High", tr: "Yüksek" }
    QualityVeryHigh { en: "Very high", tr: "Çok yüksek" }
    SectionSound { en: "Sound", tr: "Ses" }
    RowQuality { en: "Audio quality", tr: "Ses kalitesi" }
    RowQualityDesc { en: "Higher quality uses more data. Changes apply from the next song.", tr: "Yüksek kalite daha çok internet kullanır. Değişiklik sıradaki şarkıdan başlar." }
    RowNormalise { en: "Normalize volume", tr: "Ses düzeylerini eşitle" }
    RowNormaliseDesc { en: "Plays loud and quiet songs at a similar volume.", tr: "Yüksek ve alçak sesli şarkıları benzer düzeyde çalar." }
    RowDevice { en: "Output device", tr: "Çıkış aygıtı" }
    RowDeviceDesc { en: "The speakers or headphones your music plays on.", tr: "Müziğin çalacağı hoparlör ya da kulaklık." }
    On { en: "On", tr: "Açık" }
    Off { en: "Off", tr: "Kapalı" }
    CloseToTray { en: "Hide to tray", tr: "Tepsiye gizle" }
    CloseQuit { en: "Quit the app", tr: "Uygulamadan çık" }
    SectionBehavior { en: "Appearance and behavior", tr: "Görünüm ve davranış" }
    RowTheme { en: "Theme", tr: "Tema" }
    RowThemeDesc { en: "Choose your colors. “System” follows the Windows app color mode.", tr: "Renklerini seç. “Sistem”, Windows'un uygulama renk modunu izler." }
    ThemeDark { en: "Dark", tr: "Koyu" }
    ThemeLight { en: "Light", tr: "Açık" }
    ThemeSystem { en: "System", tr: "Sistem" }
    RowMotion { en: "Reduce motion", tr: "Animasyonları azalt" }
    RowMotionDesc { en: "Keeps transitions and movement to a minimum. “System” follows the Windows setting.", tr: "Geçişleri ve hareketi en aza indirir. “Sistem”, Windows'taki ayarı izler." }
    RowNowPanel { en: "Open the Now Playing panel at launch", tr: "Şimdi çalıyor panelini açık başlat" }
    RowNowPanelDesc { en: concat!("When ", app_name!(), " opens, the panel on the right is already there."), tr: concat!(app_name!(), " açılınca sağdaki panel hazır görünür.") }
    RowClose { en: "Close button", tr: "Kapat düğmesi" }
    RowCloseDesc { en: "Music keeps playing when it hides to the tray.", tr: "Tepsiye gizlenince müzik çalmaya devam eder." }
    RowHotkey { en: "Search from anywhere", tr: "Her yerden arama" }
    RowHotkeyDesc { en: "Open search with Ctrl+Shift+Space, even while you're in another app.", tr: "Ctrl+Shift+Boşluk ile başka bir uygulamadayken de arama aç." }
    AutostartFailed { en: "Couldn't change Windows' startup list.", tr: "Windows'un açılış listesi değiştirilemedi." }
    SectionStartup { en: "Startup", tr: "Başlangıç" }
    RowAutostart { en: "Start with Windows", tr: "Windows açılınca başlat" }
    RowAutostartDesc { en: concat!("When you turn on your PC, ", app_name!(), " waits hidden in the tray, ready to play."), tr: concat!("Bilgisayarını açtığında ", app_name!(), " tepside gizli, çalmaya hazır bekler.") }
    RowResume { en: "Resume where you left off", tr: "Kaldığın yerden devam et" }
    RowResumeDesc { en: "Your last song and queue are ready when it opens.", tr: "Açılınca son çalan şarkı ve sıra hazır gelir." }
    CacheUsing { en: "Using {} right now.", tr: "Şu an {} yer kaplıyor." }
    CacheSizing { en: "Calculating size…", tr: "Boyut hesaplanıyor…" }
    NotFound { en: "Not found", tr: "Bulunamadı" }
    ClearCache { en: "Clear cache", tr: "Önbelleği temizle" }
    OpenFolder { en: "Open folder", tr: "Klasörü aç" }
    ConnectedAs { en: "Connected as {}.", tr: "{} olarak bağlısın." }
    ShownHereWhenConnected { en: "Appears here once connected.", tr: "Bağlanınca burada görünür." }
    SectionStorage { en: "Storage and account", tr: "Depolama ve hesap" }
    CacheDesc { en: "Covers and songs are stored so they load fast next time. {}", tr: "Kapaklar ve şarkılar, sonraki açılışta hızlı gelsin diye saklanır. {}" }
    CacheLocation { en: "Cache location", tr: "Önbellek konumu" }
    CheckNow { en: "Check now", tr: "Şimdi denetle" }
    Hide { en: "Hide", tr: "Gizle" }
    Show { en: "Show", tr: "Göster" }
    Licenses { en: "Licenses", tr: "Lisanslar" }
    LicensesDesc { en: concat!(app_name!(), " uses open-source software."), tr: concat!(app_name!(), " açık kaynak yazılımlar kullanır.") }
    AboutDisclaimer { en: concat!(app_name!(), " is an independent project. It isn't affiliated with or endorsed by Spotify."), tr: concat!(app_name!(), " bağımsız bir projedir; Spotify ile bağlantılı değildir ve Spotify tarafından onaylanmamıştır.") }
    SettingsSubtitle { en: concat!("Make ", app_name!(), " your own"), tr: concat!(app_name_acc!(), " kendine göre ayarla") }

    // ── src/ui/mod.rs ──
    NoticeOffline { en: "You're offline. It connects when the internet is back, and then you can play.", tr: "Çevrimdışısın. İnternet gelince bağlanır, sonra çalabilirsin." }
    NoticeNoAudio { en: concat!("No audio output. Connect speakers or headphones and reopen ", app_name!(), "."), tr: concat!("Ses çıkışı yok. Hoparlör ya da kulaklık bağlayıp ", app_name_acc!(), " yeniden aç.") }
    NoticeNotTrackLink { en: "The text on the clipboard isn't a Spotify song link.", tr: "Panodaki metin bir Spotify şarkı bağlantısı değil." }
    StatusConnecting { en: "Connecting to Spotify…", tr: "Spotify'a bağlanılıyor…" }
    StatusConnected { en: "Connected: {}", tr: "Bağlandı: {}" }
    StatusFailed { en: "Couldn't connect to Spotify. It connects on its own once the internet is back.", tr: "Spotify'a bağlanılamadı. İnternet gelince kendiliğinden bağlanır." }
    StatusNoAudio { en: concat!("No audio output found. Connect your speakers or headphones and reopen ", app_name!(), "."), tr: concat!("Ses çıkışı bulunamadı. Hoparlörünü ya da kulaklığını bağlayıp ", app_name_acc!(), " yeniden aç.") }
    EmptyHint { en: "Press Ctrl+K to search, or paste a Spotify link with Ctrl+V.", tr: "Ctrl+K ile ara ya da bir Spotify bağlantısını Ctrl+V ile yapıştır." }

    // ── src/ui/player.rs ──
    PlayDenied { en: "Spotify wouldn't play this song. Press Play to try again.", tr: "Spotify bu şarkıyı vermedi. Çal'a basıp yeniden dene." }
    Reconnecting { en: "Reconnecting to Spotify…", tr: "Spotify bağlantısı yenileniyor…" }

    // ── src/update.rs ──
    UpdateReadyNotice { en: "Update ready. It will apply when you reopen the app.", tr: "Güncelleme hazır, yeniden açınca uygulanacak." }

    // ── src/system/hook.rs ──
    HotkeyLabel { en: "Ctrl+Shift+Space", tr: "Ctrl+Shift+Boşluk" }

    // ── src/system/mod.rs ──
    Paused { en: "paused", tr: "duraklatıldı" }

    // ── src/system/mod.rs ──
    HotkeyTaken { en: "{} is used by another app, so search from anywhere is off.", tr: "{} başka bir uygulamada kullanılıyor: her yerden arama kapalı." }

    // ── src/i18n/canon.rs ──
    CanonFollowed { en: "Followed", tr: "Takip edilen" }

    // ── src/engine/wire.rs ──
    EditFailedNotice { en: "One change couldn't be saved to Spotify. You can retry from the sync icon in the title bar.", tr: "Bir değişiklik Spotify'a yazılamadı. Başlıktaki eşitleme simgesinden yeniden deneyebilirsin." }
    EditQueueFailed { en: concat!("Couldn't save the change: the write queue couldn't be opened. Reopen ", app_name!(), "."), tr: concat!("Değişiklik kaydedilemedi: yazma kuyruğu açılamadı. ", app_name_acc!(), " yeniden aç.") }
    EditSaveFailed { en: "Couldn't save the change. Try again.", tr: "Değişiklik kaydedilemedi. Yeniden dene." }

    // ── src/msg.rs ──
    PlayNeedsLogin { en: "Sign in to Spotify first. Then you can play.", tr: "Spotify girişi gerekiyor. Giriş yapınca çalabilirsin." }
    PlayLoggingIn { en: "Waiting for you to sign in to Spotify in the browser. Once you're done, you can play.", tr: "Tarayıcıda Spotify girişi bekleniyor. Girişi tamamlayınca çalabilirsin." }
    PlayLoginFailed { en: "Couldn't sign in to Spotify. Try signing in again.", tr: "Spotify girişi yapılamadı. Giriş yapmayı yeniden dene." }
    PlayNeedsPremium { en: concat!("This Spotify account is free. ", app_name!(), " only plays on Premium accounts; sign in with a Premium account."), tr: concat!("Bu Spotify hesabı ücretsiz. ", app_name!(), " yalnız Premium hesapla çalar; Premium bir hesapla giriş yapabilirsin.") }
    PlayEngineStopped { en: "The audio engine stopped. You need to restart it.", tr: "Ses motoru durdu. Yeniden başlatman gerekiyor." }
    LoginTimedOut { en: "Sign-in wasn't completed within 3 minutes. If you try again, the browser opens again.", tr: "Giriş 3 dakika içinde tamamlanmadı. Yeniden denersen tarayıcı yine açılır." }
    LoginPortBusy { en: "The local port needed for sign-in (8898) is in use by another app. Close it and try again.", tr: "Giriş için gereken yerel port (8898) başka bir uygulamada açık. Onu kapatıp yeniden dene." }
    LoginNetwork { en: "Can't reach Spotify. Check your internet connection and try again.", tr: "Spotify'a ulaşılamadı. İnternet bağlantını kontrol edip yeniden dene." }
    LoginFailed { en: "Couldn't finish signing in to Spotify. Try again.", tr: "Spotify girişi tamamlanamadı. Yeniden dene." }
    EngineStopped { en: "The audio engine stopped unexpectedly. Details are in the log file; you can press Restart.", tr: "Ses motoru beklenmedik biçimde durdu. Ayrıntılar kayıt dosyasında; Yeniden başlat'a basabilirsin." }
    BtnSignIn { en: "Sign in", tr: "Giriş yap" }
    BtnCancelLogin { en: "Cancel sign-in", tr: "Girişi iptal et" }
    BtnRestart { en: "Restart", tr: "Yeniden başlat" }
    BtnOpenLogFolder { en: "Open log folder", tr: "Kayıt klasörünü aç" }
    StatusNeedsLogin { en: "You're not signed in to Spotify.", tr: "Spotify'a giriş yapmadın." }
    StatusLoggingIn { en: "Waiting for sign-in to Spotify in the browser…", tr: "Tarayıcıda Spotify girişi bekleniyor…" }
    StatusNeedsPremium { en: concat!("This Spotify account is free. ", app_name!(), " only plays on Premium accounts."), tr: concat!("Bu Spotify hesabı ücretsiz. ", app_name!(), " yalnız Premium hesapla çalar.") }
    PillNeedsLoginTitle { en: "Sign-in needed", tr: "Giriş gerekli" }
    PillNeedsLoginDetail { en: "Sign in to Spotify", tr: "Spotify'a giriş yap" }
    PillLoggingInTitle { en: "Waiting for sign-in", tr: "Giriş bekleniyor" }
    PillLoggingInDetail { en: "Approve in the browser", tr: "Tarayıcıda onayla" }
    PillLoginFailedTitle { en: "Couldn't sign in", tr: "Giriş yapılamadı" }
    PillNeedsPremiumTitle { en: "Premium required", tr: "Premium gerekli" }
    PillNeedsPremiumDetail { en: "Account is free", tr: "Hesap ücretsiz" }
    PillEngineStoppedTitle { en: "Audio engine stopped", tr: "Ses motoru durdu" }
    LibNeedsLoginTitle { en: "Spotify sign-in needed", tr: "Spotify girişi gerekli" }
    LibNeedsLoginBody { en: "Once you sign in, your library appears here.", tr: "Giriş yapınca kütüphanen burada görünür." }
    LibNeedsPremiumBody { en: concat!(app_name!(), " only plays on Spotify Premium accounts."), tr: concat!(app_name!(), " yalnız Spotify Premium hesaplarıyla çalar.") }
    SettingsLogTitle { en: "Logs", tr: "Kayıtlar" }
    SettingsLogDetail { en: concat!("If something goes wrong, the logs ", app_name!(), " writes and crash reports live here."), tr: concat!("Bir sorun olursa ", app_name_acc!(), "n yazdığı kayıtlar ve çökme raporları burada durur.") }

    // ── src/auth.rs ──
    LoginDonePage { en: concat!("Signed in to ", app_name!(), ". You can close this tab."), tr: concat!(app_name_dat!(), " giriş yapıldı. Bu sekmeyi kapatabilirsin.") }

    // ── src/engine/mod.rs ──
    NoticeRefreshing { en: "Refreshing library…", tr: "Kütüphane tazeleniyor…" }
    NoticeLibraryUpToDate { en: "Library is up to date.", tr: "Kütüphane güncel." }
    NoticeRefreshFailed { en: "Couldn't refresh your library. Check your connection and try again.", tr: "Kütüphane tazelenemedi. Bağlantını kontrol edip yeniden dene." }
    NoticeArtistTracksFailed { en: "Couldn't load the artist's songs. Try again in a moment.", tr: "Sanatçının şarkıları alınamadı. Biraz sonra yeniden dene." }
    NoticeSlowReconnecting { en: "Connection is slow. Reconnecting…", tr: "Bağlantı yavaş, yeniden bağlanılıyor…" }
    NoticeSeveralFailed { en: "Several songs in a row couldn't play. Wait a moment, then press Play.", tr: "Üst üste birkaç şarkı çalınamadı. Biraz bekleyip Çal'a bas." }
    NoticeRetryingSong { en: "Spotify didn't provide the song. Trying again…", tr: "Spotify şarkıyı vermedi, yeniden deneniyor…" }
    NoticePrevFailed { en: "The previous song couldn't play, so we moved on to this one.", tr: "Önceki şarkı çalınamadı, bu şarkıya geçildi." }
    NoticeReconnectSoon { en: "Couldn't connect to Spotify. Trying again shortly…", tr: "Spotify'a bağlanılamadı, birazdan yeniden denenecek…" }
    NoticeDeviceLost { en: "Audio device disconnected. Press Play to continue from where you left off on the current device.", tr: "Ses aygıtı ayrıldı. Çal'a basınca o anki aygıttan kaldığı yerden sürer." }
    NoticeNoDevice { en: "No audio device found. Connect speakers or headphones, then press Play.", tr: "Ses aygıtı bulunamadı. Bir hoparlör ya da kulaklık bağlayıp Çal'a bas." }

    // Arama çipleri / son çalınanlar
    SearchChipAll { en: "All", tr: "Hepsi" }
    SearchChipPodcasts { en: "Podcasts", tr: "Podcast'ler" }
    SearchChipProfiles { en: "Profiles", tr: "Profiller" }
    SearchSectionEpisodes { en: "Episodes", tr: "Bölümler" }
    SearchChipEmpty { en: "No {} found for this search.", tr: "Bu aramada {} bulunamadı." }
    SearchOpenOnSpotify { en: "Open on Spotify", tr: "Spotify'da aç" }
    SearchKindProfile { en: "Profile", tr: "Profil" }
    SearchKindPodcast { en: "Podcast", tr: "Podcast" }
    SearchKindEpisode { en: "Episode", tr: "Bölüm" }
    RecentPageTitle { en: "Recently played", tr: "Son çalınanlar" }
    RecentPageCount { en: "{} playlist or album|{} playlists and albums", tr: "{} liste ve albüm" }
    RecentEmptyTitle { en: "Nothing played yet", tr: "Henüz bir şey çalmadın" }
    RecentEmptyBody { en: "Playlists and albums you play will show up here, newest first.", tr: "Çaldığın listeler ve albümler burada, en yenisi başta durur." }
    RecentEmptyAction { en: "Browse Explore", tr: "Keşfet'e bak" }
    CmdRecentDesc { en: "Playlists and albums you played lately", tr: "Son çaldığın listeler ve albümler" }

    // Karşılama / boş durumlar (src/ui/welcome.rs)
    WelcomeTitle { en: concat!("Welcome to ", app_name!()), tr: concat!(app_name_dat!(), " hoş geldin") }
    WelcomeLine { en: "A calm, fast desktop player for your Spotify.", tr: "Spotify için sakin ve hızlı bir masaüstü çalar." }
    WelcomeSignIn { en: "Sign in with Spotify", tr: "Spotify ile giriş yap" }
    WelcomePremiumNote { en: "Spotify Premium is required to play music.", tr: "Müzik çalmak için Spotify Premium gerekir." }
    WelcomeWaitingTitle { en: "Finish signing in", tr: "Girişi tamamla" }
    WelcomeWaitingBody { en: "Approve access in your browser. This page continues on its own.", tr: "Tarayıcında erişime izin ver. Bu sayfa kendiliğinden devam eder." }
    GateOfflineTitle { en: "You're offline", tr: "Çevrimdışısın" }
    GateOfflineBody { en: concat!(app_name!(), " keeps trying on its own. Spotify's content appears here as soon as it's reachable."), tr: concat!(app_name!(), " kendiliğinden yeniden dener. Spotify'a ulaşılınca içerik burada görünür.") }
    GateTryNow { en: "Try now", tr: "Şimdi dene" }
    GateTrying { en: "Trying…", tr: "Deneniyor…" }
    GatePremiumBody { en: concat!(app_name!(), " plays music through Spotify Premium. This account is on the free plan."), tr: concat!(app_name!(), " müziği Spotify Premium üzerinden çalar. Bu hesap ücretsiz planda.") }
    GateGetPremium { en: "Get Premium", tr: "Premium'a geç" }
    GateCheckAgain { en: "Check again", tr: "Yeniden kontrol et" }
    GateStoppedBody { en: "Something went wrong with the audio engine. Restart it to continue.", tr: "Ses motorunda bir sorun çıktı. Devam etmek için yeniden başlat." }
    GateSlowTitle { en: "Taking longer than usual", tr: "Her zamankinden uzun sürüyor" }
    GateSlowBody { en: "Spotify is slow to answer. We'll keep trying, or you can try again now.", tr: "Spotify geç yanıt veriyor. Denemeye devam ediyoruz; istersen şimdi yeniden dene." }

    // Sanatçı takibi / paylaş
    ArtistFollow { en: "Follow", tr: "Takip et" }
    ArtistFollowing { en: "Following", tr: "Takip ediliyor" }
    MenuFollowArtist { en: "Follow", tr: "Takip et" }
    MenuUnfollowArtist { en: "Unfollow", tr: "Takipten çık" }
    ToastArtistFollowed { en: "Following {}", tr: "{} takip ediliyor" }
    ToastArtistUnfollowed { en: "Unfollowed {}", tr: "{} takipten çıkıldı" }
    FailFollowDenied { en: "Spotify didn't allow this change to the artists you follow.", tr: "Spotify takip ettiğin sanatçılardaki bu değişikliğe izin vermedi." }
    FailGoneArtist { en: "This artist is no longer on Spotify.", tr: "Sanatçı Spotify'da artık yok." }
    FailedFollow { en: "Couldn't follow {}", tr: "{} takip edilemedi" }
    FailedUnfollow { en: "Couldn't unfollow {}", tr: "{} takipten çıkılamadı" }
    AnArtistFallback { en: "this artist", tr: "bu sanatçı" }
    KindArtists { en: "Artists", tr: "Sanatçılar" }
    SortRecentlyFollowed { en: "Recently followed", tr: "Son takip edilen" }
    LibraryNoArtistsTitle { en: "No artists yet", tr: "Henüz sanatçı yok" }
    LibraryNoArtistsBody { en: "Follow an artist and they will show up here.", tr: "Bir sanatçıyı takip edince burada görünür." }
    MenuOpenInSpotify { en: "Open in Spotify", tr: "Spotify'da aç" }
    ToastOpeningInSpotify { en: "Opening in Spotify…", tr: "Spotify'da açılıyor…" }
    ToastOpeningInBrowser { en: "Spotify app not found. Opening in your browser…", tr: "Spotify uygulaması bulunamadı, tarayıcıda açılıyor…" }

    // Karıştırma kuralları / kısayol yardımı
    SectionShuffle { en: "Shuffle and queue", tr: "Karıştırma ve sıra" }
    RowDedupe { en: "One version of each song", tr: "Her şarkının tek sürümü" }
    RowDedupeDesc { en: "When shuffling, plays only one version of a song. A remaster, live take or radio edit won't come around again.", tr: "Karıştırırken bir şarkının yalnız bir sürümü çalar. Remaster, canlı kayıt ya da radyo sürümü yeniden çıkmaz." }
    RowSkipExplicit { en: "Skip explicit songs", tr: "Açık sözlü şarkıları atla" }
    RowSkipExplicitDesc { en: "Songs Spotify marks as explicit stay out of what plays next. A song you pick yourself still plays.", tr: "Spotify'ın açık sözlü işaretlediği şarkılar sıradakilere girmez. Kendi seçtiğin şarkı yine çalar." }
    RowNeverPlay { en: "Never play", tr: "Hiç çalma" }
    RowNeverPlayDesc { en: "Shuffle, Next and autoplay skip these artists. A song you pick yourself still plays.", tr: "Karıştırma, Sonraki ve otomatik çalma bu sanatçıları atlar. Kendi seçtiğin şarkı yine çalar." }
    NeverPlayEmpty { en: "No artists yet. Choose “Never play this artist” in a song's or an artist's menu.", tr: "Henüz sanatçı yok. Bir şarkının ya da sanatçının menüsünden “Bu sanatçıyı hiç çalma”yı seç." }
    NeverPlayAllow { en: "Allow again", tr: "Yeniden izin ver" }
    ShuffleSpreadNote { en: "While shuffling, songs by the same artist are kept apart wherever the list allows.", tr: "Karıştırırken aynı sanatçının şarkıları, liste izin verdiği ölçüde birbirinden ayrı tutulur." }
    MenuNeverPlayArtist { en: "Never play this artist", tr: "Bu sanatçıyı hiç çalma" }
    MenuNeverPlayNamed { en: "Never play {}", tr: "{} sanatçısını hiç çalma" }
    MenuAllowArtist { en: "Allow this artist again", tr: "Bu sanatçıya yeniden izin ver" }
    MenuAllowNamed { en: "Allow {} again", tr: "{} sanatçısına yeniden izin ver" }
    ToastArtistBlocked { en: "{} won't play from now on", tr: "{} artık çalınmayacak" }
    ToastArtistAllowed { en: "{} can play again", tr: "{} yeniden çalınabilir" }
    NormaliseQuiet { en: "Quiet", tr: "Sessiz" }
    NormaliseNormal { en: "Normal", tr: "Normal" }
    NormaliseLoud { en: "Loud", tr: "Yüksek" }
    ShBack { en: "Back (mouse side button too)", tr: "Geri (farenin yan tuşu da)" }
    ShForward { en: "Forward (mouse side button too)", tr: "İleri (farenin yan tuşu da)" }
    ShHelp { en: "Show this list", tr: "Bu listeyi göster" }
    ShGroupPlayback { en: "Playback", tr: "Çalma" }
    ShGroupPanels { en: "Panels and window", tr: "Paneller ve pencere" }
    ShGroupNavigate { en: "Search and go to", tr: "Ara ve git" }
    ShGroupLists { en: "Lists", tr: "Listeler" }
    ShGroupContext { en: "Moving around", tr: "Tuşlarla gezinme" }

    // Arkadaş etkinliği
    RowFriendActivity { en: "Show friend activity", tr: "Arkadaş etkinliğini göster" }
    SectionIntegrations { en: "Integrations", tr: "Entegrasyonlar" }
    RowDiscordPresence { en: "Share listening on Discord", tr: "Dinlediklerimi Discord'da paylaş" }
    RowDiscordPresenceDesc { en: "Shows your song and progress on Discord. Hidden when paused or in a private session.", tr: "Şarkını ve ilerlemeni Discord'da gösterir. Duraklatınca veya gizli oturumda gizlenir." }
    RowFriendActivityDesc { en: "Shows the song your friends listened to last. Only friends who share what they listen to appear.", tr: "Arkadaşlarının son dinlediği şarkıyı gösterir. Yalnız dinlediklerini paylaşan arkadaşlar görünür." }
    FriendsTitle { en: "Friend activity", tr: "Arkadaş etkinliği" }
    FriendsEmpty { en: "No friend activity to show yet", tr: "Gösterilecek arkadaş etkinliği yok" }
    FriendsLoading { en: "Loading friend activity…", tr: "Arkadaş etkinliği yükleniyor…" }
    FriendsFailedOffline { en: "Couldn't load friend activity", tr: "Arkadaş etkinliği alınamadı" }
    FriendsFailedDenied { en: "Friend activity isn't available for this account", tr: "Bu hesapta arkadaş etkinliği kullanılamıyor" }
    FriendsListening { en: "Listening now", tr: "Şimdi dinliyor" }
    FriendsAgoNow { en: "now", tr: "şimdi" }
    FriendsAgoMinutes { en: "{} min", tr: "{} dk" }
    FriendsAgoHours { en: "{} h", tr: "{} sa" }
    FriendsAgoDays { en: "{} d", tr: "{} g" }
    FriendsAgoWeeks { en: "{} w", tr: "{} hf" }
    FriendsMenuProfile { en: "Open profile in Spotify", tr: "Profili Spotify'da aç" }

    // Kütüphane düzeni
    SortAdded { en: "Date added", tr: "Eklenme tarihi" }
    SortCreator { en: "Creator", tr: "Yaratıcı" }
    MenuPin { en: "Pin to top", tr: "En üste sabitle" }
    MenuUnpin { en: "Unpin", tr: "Sabitlemeyi kaldır" }
    ToastPinned { en: "Pinned to the top of your library", tr: "Kütüphanenin en üstüne sabitlendi" }
    ToastUnpinned { en: "Unpinned", tr: "Sabitleme kaldırıldı" }
    KindFolder { en: "Folder", tr: "Klasör" }
    FolderLists { en: "{} playlist|{} playlists", tr: "{} liste" }
    MenuExpandFolder { en: "Expand", tr: "Genişlet" }
    MenuCollapseFolder { en: "Collapse", tr: "Daralt" }
    FolderUpTip { en: "Up one folder", tr: "Bir üst klasör" }
    FolderEmptyTitle { en: "This folder is empty", tr: "Bu klasör boş" }
    FolderEmptyBody { en: "Playlists you put in this folder in Spotify show up here.", tr: "Spotify'da bu klasöre koyduğun listeler burada görünür." }
    // Liste açıklaması, ortak liste, klasör yazmaları.
    MenuAddDescription { en: "Add a description", tr: "Açıklama ekle" }
    MenuEditDescription { en: "Edit description", tr: "Açıklamayı düzenle" }
    PromptDescriptionTitle { en: "Playlist description", tr: "Liste açıklaması" }
    PromptDescriptionHint { en: "Say something about this playlist", tr: "Bu liste hakkında bir şey yaz" }
    PromptChars { en: "{}/{}", tr: "{}/{}" }
    ToastDescriptionSaved { en: "Description saved", tr: "Açıklama kaydedildi" }
    MenuMakeCollaborative { en: "Make collaborative", tr: "Ortak listeye çevir" }
    MenuMakePrivate { en: "Stop collaborating", tr: "Ortak listeyi kapat" }
    SharedTip { en: "Collaborative playlist", tr: "Ortak liste" }
    ToastCollaborativeOn { en: "Anyone with the link can add songs", tr: "Bağlantıya sahip olanlar şarkı ekleyebilir" }
    ToastCollaborativeOff { en: "This playlist is private again", tr: "Liste yeniden özel" }
    NewFolder { en: "New folder", tr: "Yeni klasör" }
    PromptNewFolderTitle { en: "New folder", tr: "Yeni klasör" }
    PromptRenameFolderTitle { en: "Rename folder", tr: "Klasörü yeniden adlandır" }
    PromptFolderHint { en: "Folder name", tr: "Klasörün adı" }
    MenuRenameFolder { en: "Rename folder", tr: "Klasörü yeniden adlandır" }
    MenuDeleteFolder { en: "Delete folder", tr: "Klasörü sil" }
    MenuMoveToFolder { en: "Move to folder", tr: "Klasöre taşı" }
    MenuNewFolderDots { en: "New folder…", tr: "Yeni klasör…" }
    MenuMoveToRoot { en: "Move out of folder", tr: "Klasörden çıkar" }
    ToastFolderCreated { en: "Folder “{}” created", tr: "“{}” klasörü oluşturuldu" }
    ToastFolderRenamed { en: "Folder renamed", tr: "Klasör yeniden adlandırıldı" }
    ToastFolderDeleted { en: "Folder deleted, its playlists moved out", tr: "Klasör silindi, içindekiler dışarı çıktı" }
    ToastMovedToFolder { en: "Moved to “{}”", tr: "“{}” klasörüne taşındı" }
    ToastMovedToRoot { en: "Moved out of the folder", tr: "Klasörden çıkarıldı" }
    FailMetaDenied { en: "Couldn't save this playlist's settings", tr: "Listenin ayarları kaydedilemedi" }
    FailFolderDenied { en: "Couldn't save this folder change", tr: "Klasör değişikliği kaydedilemedi" }
    FailGoneFolder { en: "That folder is no longer in your library", tr: "O klasör artık kütüphanende değil" }
    FailedDescription { en: "Couldn't save the description of {}", tr: "{} listesinin açıklaması kaydedilemedi" }
    FailedCollaborative { en: "Couldn't change who can add songs to {}", tr: "{} listesinin ortak ayarı değiştirilemedi" }
    FailedFolderCreate { en: "Couldn't create the folder “{}”", tr: "“{}” klasörü oluşturulamadı" }
    FailedFolderRestore { en: "Couldn't restore the folder “{}”", tr: "“{}” klasörü geri getirilemedi" }
    FailedFolderRename { en: "Couldn't rename the folder to “{}”", tr: "Klasör “{}” olarak adlandırılamadı" }
    FailedFolderDelete { en: "Couldn't delete the folder “{}”", tr: "“{}” klasörü silinemedi" }
    FailedFolderMove { en: "Couldn't move {} into the folder", tr: "{} klasöre taşınamadı" }
    ColDateAdded { en: "Date added", tr: "Eklenme tarihi" }
    AddedNow { en: "Just now", tr: "Az önce" }
    AddedMinutes { en: "{} minute ago|{} minutes ago", tr: "{} dk önce" }
    AddedHours { en: "{} hour ago|{} hours ago", tr: "{} saat önce" }
    AddedDays { en: "{} day ago|{} days ago", tr: "{} gün önce" }
    AddedWeeks { en: "{} week ago|{} weeks ago", tr: "{} hafta önce" }
    DateMonthDayYear { en: "{0} {1}, {2}", tr: "{1} {0} {2}" }
    MonthJan { en: "Jan", tr: "Oca" }
    MonthFeb { en: "Feb", tr: "Şub" }
    MonthMar { en: "Mar", tr: "Mar" }
    MonthApr { en: "Apr", tr: "Nis" }
    MonthMay { en: "May", tr: "May" }
    MonthJun { en: "Jun", tr: "Haz" }
    MonthJul { en: "Jul", tr: "Tem" }
    MonthAug { en: "Aug", tr: "Ağu" }
    MonthSep { en: "Sep", tr: "Eyl" }
    MonthOct { en: "Oct", tr: "Eki" }
    MonthNov { en: "Nov", tr: "Kas" }
    MonthDec { en: "Dec", tr: "Ara" }

    // Künye ve söz çevirisi
    MenuShowCredits { en: "Show credits", tr: "Künyeyi göster" }
    CreditsTitle { en: "Credits", tr: "Künye" }
    CreditsPerformers { en: "Performed by", tr: "Seslendiren" }
    CreditsWriters { en: "Written by", tr: "Yazan" }
    CreditsProducers { en: "Produced by", tr: "Yapımcı" }
    CreditsMixing { en: "Mixing engineer", tr: "Miks mühendisi" }
    CreditsMastering { en: "Mastering engineer", tr: "Mastering mühendisi" }
    CreditsRecording { en: "Recording engineer", tr: "Kayıt mühendisi" }
    CreditsComposer { en: "Composer", tr: "Besteci" }
    CreditsLyricist { en: "Lyricist", tr: "Söz yazarı" }
    CreditsArranger { en: "Arranger", tr: "Aranjör" }
    CreditsSource { en: "Source", tr: "Kaynak" }
    CreditsLoading { en: "Loading credits…", tr: "Künye yükleniyor…" }
    CreditsEmpty { en: "No credits for this song", tr: "Bu şarkı için künye yok" }
    CreditsFailedOffline { en: "Couldn't load the credits", tr: "Künye alınamadı" }
    CreditsFailedBusy { en: "Spotify is busy. Try again in a moment.", tr: "Spotify şu an yoğun. Biraz sonra yeniden dene." }
    CreditsFailedDenied { en: "Credits aren't available for this account", tr: "Bu hesapta künye kullanılamıyor" }
    LyricsTranslation { en: "Translation", tr: "Çeviri" }
    HideLyricsTranslation { en: "Hide translation", tr: "Çeviriyi gizle" }
    ShowLyricsTranslation { en: "Show translation", tr: "Çeviriyi göster" }

    // Kütüphane araçları
    ToolsTitle { en: "Tools", tr: "Araçlar" }
    ToolsSubtitle { en: "Find duplicates, merge and split playlists, sort, back up, and see what you listen to", tr: "Yinelenenleri bul, listeleri birleştir ve böl, sırala, yedekle, dinlediklerini gör" }
    CmdToolsDesc { en: "Duplicates, merge and split, sort, backup, stats", tr: "Yinelenenler, birleştir / böl, sıralama, yedek, istatistik" }
    ToolsDupes { en: "Duplicates", tr: "Yinelenenler" }
    ToolsMergeSplit { en: "Merge and split", tr: "Birleştir / böl" }
    ToolsSort { en: "Sort", tr: "Sıralama" }
    ToolsBackup { en: "Backup", tr: "Yedek" }
    ToolsStats { en: "Stats", tr: "İstatistik" }
    ToolsPickList { en: "Pick a playlist", tr: "Bir liste seç" }
    ToolsPickLists { en: "Pick the playlists to merge", tr: "Birleştirilecek listeleri seç" }
    ToolsPickedCount { en: "{} selected", tr: "{} seçili" }
    ToolsNoOwnLists { en: "You have no playlists of your own to change yet.", tr: "Henüz değiştirebileceğin kendi listen yok." }
    ToolsLoading { en: "Loading…", tr: "Yükleniyor…" }
    ToolsLoadFailed { en: "Couldn't load this playlist. Try again in a moment.", tr: "Bu liste alınamadı. Biraz sonra yeniden dene." }
    ToolsAndMore { en: "and {} more", tr: "ve {} tane daha" }
    ToolsDupesIntro { en: "Finds the same song twice, and different versions of one song (remaster, live, radio edit). One of each stays; you choose what to remove.", tr: "Aynı şarkıyı iki kez ve bir şarkının farklı sürümlerini (remaster, canlı, radyo düzenlemesi) bulur. Her birinden biri kalır; neyin kalkacağına sen karar verirsin." }
    ToolsDupesPick { en: "Pick a playlist to check it.", tr: "Kontrol etmek için bir liste seç." }
    ToolsDupesNoneTitle { en: "No duplicates", tr: "Yinelenen yok" }
    ToolsDupesNoneBody { en: "Every song in this playlist is there once.", tr: "Bu listedeki her şarkı bir kez geçiyor." }
    ToolsDupeSets { en: "{} duplicate set|{} duplicate sets", tr: "{} yinelenen küme" }
    ToolsDupeExtra { en: "{} extra song|{} extra songs", tr: "{} fazla şarkı" }
    ToolsDupesKinds { en: "Same song: {0} · Versions: {1}", tr: "Aynı şarkı: {0} · Sürümler: {1}" }
    ToolsDupSame { en: "Same song", tr: "Aynı şarkı" }
    ToolsDupVersions { en: "Versions of one song", tr: "Bir şarkının sürümleri" }
    ToolsDupRemovable { en: "{} to remove", tr: "{} kaldırılabilir" }
    ToolsDupKeep { en: "Stays", tr: "Kalır" }
    ToolsDupesHint { en: "One song stays in every set. Untick what you want to keep. You can undo with Ctrl+Z.", tr: "Her kümede bir şarkı kalır. Kalmasını istediklerinin işaretini kaldır. Ctrl+Z ile geri alabilirsin." }
    ToolsDupesRemove { en: "Remove selected ({})", tr: "Seçilenleri kaldır ({})" }
    ToolsConfirmTitle { en: "{} changes will be written to Spotify", tr: "Spotify'a {} değişiklik yazılacak" }
    ToolsConfirmApply { en: "Go ahead", tr: "Uygula" }
    ToolsConfirmBodyDupes { en: "These songs are removed from the playlist. You can undo with Ctrl+Z.", tr: "Bu şarkılar listeden çıkarılır. Ctrl+Z ile geri alabilirsin." }
    ToolsConfirmBodyMerge { en: "A new playlist is created and filled. Your source playlists stay as they are. You can undo with Ctrl+Z.", tr: "Yeni bir liste açılır ve doldurulur. Kaynak listelerine dokunulmaz. Ctrl+Z ile geri alabilirsin." }
    ToolsConfirmBodySplit { en: "New playlists are created and filled. Your source playlist stays as it is. You can undo with Ctrl+Z.", tr: "Yeni listeler açılır ve doldurulur. Kaynak listene dokunulmaz. Ctrl+Z ile geri alabilirsin." }
    ToolsConfirmBodySort { en: "The playlist is reordered on Spotify one move at a time. You can undo with Ctrl+Z.", tr: "Liste Spotify'da tek tek taşınarak yeniden sıralanır. Ctrl+Z ile geri alabilirsin." }
    ToolsMergeIntro { en: "Combine two or more playlists into a new one, or split one playlist into several. Your source playlists are never changed or deleted.", tr: "İki ya da daha çok listeyi yeni bir listede birleştir ya da bir listeyi birkaçına böl. Kaynak listelerin hiç değişmez, silinmez." }
    ToolsMode { en: "What to do", tr: "Ne yapılsın" }
    ToolsModeMerge { en: "Merge", tr: "Birleştir" }
    ToolsModeSplit { en: "Split", tr: "Böl" }
    ToolsMergeTooFew { en: "Merging needs at least two playlists.", tr: "Birleştirmek için en az iki liste gerekir." }
    ToolsDropDupes { en: "Drop duplicates", tr: "Yinelenenleri at" }
    ToolsDedupeOff { en: "Off", tr: "Kapalı" }
    ToolsDedupeSame { en: "Same song", tr: "Aynı şarkı" }
    ToolsDedupeVersions { en: "With versions", tr: "Sürümler dahil" }
    ToolsSplitBy { en: "Split by", tr: "Neye göre böl" }
    ToolsByArtist { en: "Artist", tr: "Sanatçı" }
    ToolsByAlbum { en: "Album", tr: "Albüm" }
    ToolsByYear { en: "Year added", tr: "Eklendiği yıl" }
    ToolsByYearNote { en: "Release years aren't known here, so this splits by the year each song was added to the playlist.", tr: "Çıkış yılı burada bilinmiyor; şarkıların listeye eklendiği yıla göre bölünür." }
    ToolsPreview { en: "Preview", tr: "Önizle" }
    ToolsReviewTitle { en: "Check before creating", tr: "Oluşturmadan önce bak" }
    ToolsReviewHint { en: "Nothing is written to Spotify until you confirm.", tr: "Onaylayana kadar Spotify'a hiçbir şey yazılmaz." }
    ToolsListName { en: "Playlist name", tr: "Liste adı" }
    ToolsListPrefix { en: "Names start with", tr: "Adların başı" }
    ToolsMergeDropped { en: "Duplicates dropped: {}", tr: "Atılan yinelenen: {}" }
    ToolsNewLists { en: "{} new playlist|{} new playlists", tr: "{} yeni liste" }
    ToolsSourcesStay { en: "The source playlists stay exactly as they are.", tr: "Kaynak listeler olduğu gibi kalır." }
    ToolsCreateList { en: "Create playlist", tr: "Listeyi oluştur" }
    ToolsCreateLists { en: "Create playlists", tr: "Listeleri oluştur" }
    ToolsSplitOther { en: "Other", tr: "Diğer" }
    ToolsSplitDone { en: "{} playlist created|{} playlists created", tr: "{} liste oluşturuldu" }
    ToolsSortIntro { en: "Reorder a playlist for real on Spotify: by name, artist, album, date added or length.", tr: "Bir listeyi Spotify'da gerçekten yeniden sırala: ada, sanatçıya, albüme, eklenme tarihine ya da süreye göre." }
    ToolsSortBy { en: "Sort by", tr: "Ölçüt" }
    ToolsByName { en: "Name", tr: "Ad" }
    ToolsByDuration { en: "Length", tr: "Süre" }
    ToolsSortDir { en: "Direction", tr: "Yön" }
    ToolsAsc { en: "Ascending", tr: "Artan" }
    ToolsDesc { en: "Descending", tr: "Azalan" }
    ToolsSortPick { en: "Pick a playlist to see how it would be sorted.", tr: "Nasıl sıralanacağını görmek için bir liste seç." }
    ToolsSortDatesLoading { en: "Reading when each song was added…", tr: "Eklenme tarihleri okunuyor…" }
    ToolsSortAlready { en: "Already in this order", tr: "Liste zaten bu sırada" }
    ToolsSortMoves { en: "Moves needed: {}", tr: "Gereken taşıma: {}" }
    ToolsSortOf { en: "Songs in the playlist: {}", tr: "Listedeki şarkı: {}" }
    ToolsSortTooMany { en: "This order needs {0} moves. At most {1} are made at once so Spotify isn't flooded. Pick a smaller playlist.", tr: "Bu sıra {0} taşıma ister. Spotify'ı boğmamak için tek seferde en çok {1} taşıma yapılır. Daha küçük bir liste seç." }
    ToolsSortHint { en: "Spotify gets the moves one at a time. You can undo with Ctrl+Z.", tr: "Taşımalar Spotify'a tek tek yazılır. Ctrl+Z ile geri alabilirsin." }
    ToolsSortApply { en: "Sort · {} move|Sort · {} moves", tr: "Sırala · {} taşıma" }
    ToolsSortDone { en: "Sorted · {} move|Sorted · {} moves", tr: "Sıralandı · {} taşıma" }
    ToolsBackupIntro { en: "Saves your library to a file on this computer: playlists with their song addresses, folders, pins and followed artists.", tr: "Kütüphaneni bu bilgisayardaki bir dosyaya kaydeder: listeler ve şarkı adresleri, klasörler, sabitlemeler ve takip ettiğin sanatçılar." }
    ToolsBackupLists { en: "Playlists: {}", tr: "Liste: {}" }
    ToolsBackupFolders { en: "Folders: {}", tr: "Klasör: {}" }
    ToolsBackupPinned { en: "Pinned: {}", tr: "Sabit: {}" }
    ToolsBackupArtists { en: "Artists: {}", tr: "Sanatçı: {}" }
    ToolsBackupNoRestore { en: "This version only exports. Restoring from a backup isn't available yet.", tr: "Bu sürüm yalnız dışa aktarır. Yedekten geri yükleme henüz yok." }
    ToolsBackupSave { en: "Save backup…", tr: "Yedeği kaydet…" }
    ToolsBackupDialogTitle { en: "Save library backup", tr: "Kütüphane yedeğini kaydet" }
    ToolsBackupChoosing { en: "Choose where to save…", tr: "Kaydedilecek yeri seç…" }
    ToolsBackupWriting { en: "Writing…", tr: "Yazılıyor…" }
    ToolsBackupFailed { en: "Couldn't write the file. Pick another place and try again.", tr: "Dosya yazılamadı. Başka bir yer seçip yeniden dene." }
    ToolsBackupDone { en: "Backup saved", tr: "Yedek kaydedildi" }
    ToolsBackupCounts { en: "Playlists: {0} · Song addresses: {1}", tr: "Liste: {0} · Şarkı adresi: {1}" }
    ToolsBackupMissing { en: "Playlists not downloaded yet (names only in the file): {}", tr: "Henüz indirilmemiş listeler (dosyada yalnız ad): {}" }
    ToolsBackupShow { en: "Show in folder", tr: "Klasörde göster" }
    ToolsStatsIntro { en: "What you play here, counted on this computer. A song counts once you've heard 30 seconds of it, or half if it's shorter.", tr: "Burada çaldıkların bu bilgisayarda sayılır. Bir şarkı 30 saniye (kısaysa yarısı) dinlenince sayılır." }
    ToolsRange4w { en: "Last 4 weeks", tr: "Son 4 hafta" }
    ToolsRange6m { en: "Last 6 months", tr: "Son 6 ay" }
    ToolsRangeAll { en: "All time", tr: "Tümü" }
    ToolsPlays { en: "{} play|{} plays", tr: "{} çalma" }
    ToolsPlaysCaption { en: "plays counted", tr: "sayılan çalma" }
    ToolsListened { en: "listened", tr: "dinleme süresi" }
    ToolsMinutes { en: "{} min", tr: "{} dk" }
    ToolsHoursMinutes { en: "{0} h {1} min", tr: "{0} sa {1} dk" }
    ToolsHours { en: "When you listen", tr: "Hangi saatlerde dinliyorsun" }
    ToolsPeakHour { en: "Most around {}", tr: "En çok {} civarında" }
    ToolsTopTracks { en: "Top songs", tr: "En çok dinlenen şarkılar" }
    ToolsTopArtists { en: "Top artists", tr: "En çok dinlenen sanatçılar" }
    ToolsTopAlbums { en: "Top albums", tr: "En çok dinlenen albümler" }
    ToolsStatsEmptyTitle { en: "Nothing counted yet", tr: "Henüz sayılan bir şey yok" }
    ToolsStatsEmptyBody { en: "Songs you play here show up once you've heard 30 seconds of them.", tr: "Burada çaldığın şarkılar, 30 saniye dinlenince görünür." }
    ToolsStatsLocal { en: "Kept only on this computer. Nothing is sent to Spotify, and nothing is written during a private session.", tr: "Yalnız bu bilgisayarda tutulur. Spotify'a hiçbir şey gönderilmez, gizli oturumda hiçbir şey yazılmaz." }

    // Ekolayzer
    SectionEqualizer { en: "Equalizer", tr: "Ekolayzer" }
    EqEnable { en: "Use the equalizer", tr: "Ekolayzeri kullan" }
    EqEnableDesc { en: "Shape the sound to your taste. While it's off, the audio isn't touched.", tr: "Sesi zevkine göre biçimlendir. Kapalıyken ses hiç değişmez." }
    EqPreamp { en: "Preamp", tr: "Ön kazanç" }
    EqReset { en: "Reset", tr: "Sıfırla" }
    EqPresetFlat { en: "Flat", tr: "Düz" }
    EqPresetBass { en: "Bass boost", tr: "Bas artır" }
    EqPresetTreble { en: "Treble boost", tr: "Tiz artır" }
    EqPresetVocal { en: "Vocal", tr: "Vokal" }
    EqPresetAcoustic { en: "Acoustic", tr: "Akustik" }
    EqHeadphoneTitle { en: "Headphone profile", tr: "Kulaklık profili" }
    EqHeadphoneDesc { en: "Pick your headphones to get a ready-made correction curve.", tr: "Kulaklığını seç, hazır bir düzeltme eğrisi gelsin." }
    EqSearchPlaceholder { en: "Search brand or model", tr: "Marka veya model ara" }
    EqSearchLoading { en: "Getting headphone profiles…", tr: "Kulaklık profilleri alınıyor…" }
    EqSearchFailed { en: "Couldn't get the headphone profiles.", tr: "Kulaklık profilleri alınamadı." }
    EqRetry { en: "Try again", tr: "Yeniden dene" }
    EqSearchNone { en: "No headphones match “{}”", tr: "“{}” ile eşleşen kulaklık yok" }
    EqSearchMore { en: "Showing the first {}. Type more to narrow it down.", tr: "İlk {} gösteriliyor. Daraltmak için biraz daha yaz." }
    EqProfileMeasurement { en: "Measurement: {}", tr: "Ölçüm: {}" }
    EqProfileFit { en: "Fitted to the 10 bands, average deviation {} dB", tr: "10 banda yaklaştırıldı, ortalama sapma {} dB" }
    EqProfileRemove { en: "Remove", tr: "Kaldır" }
    EqOpraBlurb { en: "Headphone profiles come from OPRA, an open community database started by Roon Labs. They are downloaded when you first search.", tr: "Kulaklık profilleri, Roon Labs'in başlattığı açık topluluk veritabanı OPRA'dan gelir. İlk aramada indirilir." }
    LicOpra { en: "Headphone equalizer profiles (downloaded when used)", tr: "Kulaklık ekolayzer profilleri (kullanılınca indirilir)" }

    // Radyo, gizleme, gizli oturum
    MenuStartRadio { en: "Start radio", tr: "Radyoyu başlat" }
    ToastRadioStarting { en: "Starting radio…", tr: "Radyo başlıyor…" }
    NoticeRadioFailed { en: "Couldn't start the radio. Try again in a moment.", tr: "Radyo başlatılamadı. Biraz sonra yeniden dene." }
    NoticeRadioLimited { en: "Spotify is limiting radio requests right now. Try again in a few minutes.", tr: "Spotify radyo isteklerini şu an sınırlıyor. Birkaç dakika sonra yeniden dene." }
    NoticeRadioDenied { en: "Radio isn't available for this account.", tr: "Bu hesapta radyo açılamadı." }
    QueueNextFromRadio { en: "Next from radio: {}", tr: "Sıradaki: {} radyosu" }
    QueueNextFromRadioPlain { en: "Next from radio", tr: "Sıradaki: radyo" }
    CmdStartRadio { en: "Start radio from the current song", tr: "Çalan şarkının radyosunu başlat" }
    CmdStartRadioDesc { en: "Similar songs, endless", tr: "Benzer şarkılar, bitmeyen akış" }
    MenuHideSong { en: "Hide this song", tr: "Bu şarkıyı gizle" }
    MenuHideSongs { en: "Hide {} songs", tr: "{} şarkıyı gizle" }
    MenuNeverPlayAnywhere { en: "Never play anywhere", tr: "Hiçbir yerde çalma" }
    MenuShowSong { en: "Show", tr: "Göster" }
    MenuShowSongs { en: "Show {} songs", tr: "{} şarkıyı göster" }
    ToastHiddenHere { en: "{} song hidden in this list|{} songs hidden in this list", tr: "{} şarkı bu listede gizlendi" }
    ToastHiddenEverywhere { en: "{} song won't play anywhere|{} songs won't play anywhere", tr: "{} şarkı hiçbir yerde çalınmayacak" }
    ToastHiddenShown { en: "{} song shown again|{} songs shown again", tr: "{} şarkı yeniden gösteriliyor" }
    RowHidden { en: "Hidden songs", tr: "Gizlenen şarkılar" }
    RowHiddenDesc { en: "Shuffle, Next and radio skip these songs. Kept only on this computer; your Spotify account doesn't change.", tr: "Karıştırma, Sonraki ve radyo bu şarkıları atlar. Yalnız bu bilgisayarda tutulur; Spotify hesabın değişmez." }
    HiddenEmpty { en: "Nothing hidden. Choose “Hide this song” in a song's menu.", tr: "Gizlenen şarkı yok. Bir şarkının menüsünden “Bu şarkıyı gizle”yi seç." }
    HiddenShow { en: "Show", tr: "Göster" }
    HiddenEverywhere { en: "Everywhere", tr: "Her yerde" }
    HiddenInList { en: "In {}", tr: "{} içinde" }
    HiddenInUnknown { en: "In a list", tr: "Bir listede" }
    HiddenMore { en: "and {} more", tr: "ve {} tane daha" }
    MenuPrivateSession { en: "Private session", tr: "Gizli oturum" }
    CmdPrivateSession { en: "Private session", tr: "Gizli oturum" }
    CmdPrivateOn { en: "Turn on private session", tr: "Gizli oturumu aç" }
    CmdPrivateOff { en: "Turn off private session", tr: "Gizli oturumu kapat" }
    CmdPrivateSessionDesc { en: concat!("Nothing is added to ", app_name!(), "'s own history"), tr: concat!(app_name!(), " kendi geçmişine yazmaz") }
    PrivateBadgeTip { en: "Private session is on. Click to turn it off.", tr: "Gizli oturum açık. Kapatmak için tıkla." }
    ToastPrivateOn { en: concat!("Private session on. ", app_name!(), " won't add to its own history or stats."), tr: concat!("Gizli oturum açık. ", app_name!(), " kendi geçmişine ve istatistiğine yazmaz.") }
    ToastPrivateOff { en: "Private session off", tr: "Gizli oturum kapandı" }
    RowPrivate { en: "Private session", tr: "Gizli oturum" }
    RowPrivateDesc { en: concat!("While on, Recently played and your local stats aren't written, and a badge shows in the title bar. Where you left off is still saved. ", app_name!(), " doesn't report listening to Spotify either way. It ends when you quit ", app_name!(), "."), tr: concat!("Açıkken Son çalınanlar ve yerel istatistik yazılmaz, başlık çubuğunda bir simge görünür. Kaldığın yer yine kaydedilir. ", app_name!(), " zaten Spotify'a dinleme bildirmez. ", app_name_acc!(), " kapatınca biter.") }

    // Crossfade
    RowCrossfade { en: "Crossfade", tr: "Şarkılar arası geçiş" }
    RowCrossfadeDesc { en: "Songs blend as one ends. Same album stays gapless.", tr: "Şarkılar yumuşakça bağlanır. Albüm içi boşluksuz." }
    CrossfadeSec2 { en: "2 s", tr: "2 sn" }
    CrossfadeSec5 { en: "5 s", tr: "5 sn" }
    CrossfadeSec8 { en: "8 s", tr: "8 sn" }
    CrossfadeSec12 { en: "12 s", tr: "12 sn" }

    // Karaoke
    Karaoke { en: "Karaoke", tr: "Karaoke" }
    KaraokeOnTip { en: "Karaoke: reduce the vocals", tr: "Karaoke: vokali azalt" }
    KaraokeOffTip { en: "Turn karaoke off", tr: "Karaoke'yu kapat" }
    KaraokeAmount { en: "Vocal reduction", tr: "Vokal azaltma" }

    // AutoMix
    AutoMixName { en: "AutoMix", tr: "AutoMix" }
    AutoMixNote { en: "Picks where and how long to blend from how each song ends. It also lines up the beat for songs you've already played here.", tr: "Geçişin yerini ve süresini şarkının bitişine bakarak seçer. Burada daha önce çaldığın şarkılarda vuruşları da hizalar." }
    // Connect
    SectionConnect { en: "Spotify Connect", tr: "Spotify Connect" }
    RowConnect { en: "Spotify Connect", tr: "Spotify Connect" }
    RowConnectDesc { en: "Play from your phone: pick this computer in Spotify's device list and the music plays here.", tr: "Telefondan çal: Spotify'ın cihaz listesinden bu bilgisayarı seç, müzik burada çalsın." }
    ConnectExperimental { en: "Experimental", tr: "Deneysel" }
    RowConnectName { en: "Device name", tr: "Cihaz adı" }
    RowConnectNameDesc { en: "How this computer appears in Spotify's device list.", tr: "Bu bilgisayarın Spotify cihaz listesindeki adı." }
    PromptDeviceNameTitle { en: "Device name", tr: "Cihaz adı" }
    ConnectPlayingOn { en: "Playing on {}", tr: "{} cihazında çalıyor" }
    ConnectTransferHere { en: "Play here", tr: "Buraya aktar" }
    ConnectTransferHereTip { en: "Move playback to this computer", tr: "Çalmayı bu bilgisayara aktar" }

    // Friends v2: gruplanmış, kapaklı liste ve durum ekranları
    FriendsGroupLive { en: "Listening now", tr: "Şimdi dinliyor" }
    FriendsGroupEarlier { en: "Earlier", tr: "Daha önce" }
    FriendsPlayTip { en: "Play", tr: "Çal" }
    FriendsTimeAt { en: "{}, {}", tr: "{} {}" }
    FriendsEmptyBody { en: "Friends who share what they listen to will show up here.", tr: "Dinlediklerini paylaşan arkadaşların burada görünür." }
    FriendsLoadingBody { en: "This only takes a moment.", tr: "Bu birkaç saniye sürer." }
    FriendsOfflineBody { en: "Check your connection. We will try again soon.", tr: "Bağlantını kontrol et. Biraz sonra yeniden denenecek." }
    FriendsBusyTitle { en: "Spotify is busy", tr: "Spotify şu an yoğun" }
    FriendsBusyBody { en: "Trying again in a few minutes.", tr: "Birkaç dakika sonra yeniden denenecek." }
    FriendsDeniedBody { en: "Spotify doesn't share it with this account.", tr: "Spotify bunu bu hesapla paylaşmıyor." }

}
