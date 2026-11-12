//! Terminal çalar: tek bir şarkıyı pencere açmadan çalar.
//!
//!   openify <şarkı bağlantısı | spotify:track:ID | ID>

use std::time::{Duration, Instant};

use anyhow::Result;
use librespot::{
    metadata::{Metadata, Track},
    playback::{
        config::{Bitrate, PlayerConfig},
        player::PlayerEvent,
    },
};

use crate::{auth, engine, link::parse_track};

pub async fn play(input: &str) -> Result<()> {
    let track = parse_track(input)?;
    anyhow::ensure!(engine::has_output_device(), "Ses çıkışı bulunamadı");

    // 1–2. Giriş ve oturum: kayıtlı giriş varsa tarayıcı açılmaz.
    let session = auth::connect(engine::audio_cache()).await?;
    println!("Bağlandı: {}", session.username());

    // 3. Çalar: Adım 1 kararları (320 kbps, normalleştirme açık); uygulamayla
    // aynı ses çıkışı (`engine::new_player`).
    let player_config = PlayerConfig {
        bitrate: Bitrate::Bitrate320,
        normalisation: true,
        ..PlayerConfig::default()
    };
    let (player, out) = engine::new_player(player_config, session.clone(), engine::AudioLevels::new(), 1.0, 0);

    // Şarkı adını göster (başarısız olursa çalmaya engel değil).
    match Track::get(&session, &track).await {
        Ok(t) => {
            let artists: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
            println!("Şarkı: {} — {}", artists.join(", "), t.name);
        }
        Err(e) => println!("Şarkı bilgisi alınamadı: {e}"),
    }

    let mut events = player.get_player_event_channel();
    player.load(track, true, 0);
    println!("Yükleniyor...");

    // Çaların olaylarını izle: ses gerçekten başladı mı, şarkı yüklenemedi mi?
    let mut started = false;
    let result = loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(PlayerEvent::Playing { .. }) if !started => {
                    started = true;
                    println!("▶ Ses başladı. Durdurmak için Ctrl+C.");
                }
                Some(PlayerEvent::Unavailable { .. }) => {
                    break Err(anyhow::anyhow!("Şarkı yüklenemedi (Spotify ses dosyasını vermedi)"));
                }
                Some(PlayerEvent::EndOfTrack { .. }) | None => {
                    // Çözme bitti; kuyruktaki son ses (~0,1 sn) de çalsın.
                    let t = Instant::now();
                    while out.queued_frames() > 0 && t.elapsed() < Duration::from_secs(2) {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    println!("Bitti.");
                    break Ok(());
                }
                _ => {}
            },
            _ = tokio::signal::ctrl_c() => {
                println!("\nDurduruldu.");
                break Ok(());
            }
        }
    };

    player.stop();
    session.shutdown();
    result
}
