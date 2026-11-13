use super::*;

fn track() -> TrackInfo {
    TrackInfo {
        uri: "spotify:track:test".into(),
        name: "Test song".into(),
        artists: "Test artist".into(),
        duration_ms: 180_000,
        cover_url: Some("https://i.scdn.co/image/test".into()),
    }
}

#[test]
fn frame_has_little_endian_header_and_exact_body() {
    assert_eq!(
        encode_frame(1, b"{}"),
        vec![1, 0, 0, 0, 2, 0, 0, 0, 123, 125]
    );
}

#[test]
fn fragmented_and_concatenated_frames_are_preserved() {
    let mut bytes = vec![1, 0, 0];
    assert!(decode_frame(&mut bytes).unwrap().is_none());
    bytes.extend_from_slice(&[0, 2, 0, 0, 0, 123]);
    assert!(decode_frame(&mut bytes).unwrap().is_none());
    bytes.extend_from_slice(&[125, 3, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decode_frame(&mut bytes).unwrap(), Some((1, b"{}".to_vec())));
    assert_eq!(decode_frame(&mut bytes).unwrap(), Some((3, vec![])));
    assert!(bytes.is_empty());
}

#[test]
fn oversized_and_unknown_frames_are_rejected_before_body_allocation() {
    for mut bytes in [vec![1, 0, 0, 0, 1, 0, 1, 0], vec![5, 0, 0, 0, 0, 0, 0, 0]] {
        assert!(decode_frame(&mut bytes).is_err());
    }
}

#[test]
fn listening_activity_uses_track_cover_and_position() {
    assert_eq!(
        activity(&track(), 42_000, 1_000),
        json!({
            "type":2,"status_display_type":2,"details":"Test song","state":"Test artist",
            "timestamps":{"start":958,"end":1138},
            "assets":{"large_image":"https://i.scdn.co/image/test","large_text":"Test song"}
        })
    );
}

#[test]
fn clamps_position_and_uses_static_cover_for_missing_or_untrusted_urls() {
    let mut t = track();
    for cover in [None, Some("https://example.com/cover".into())] {
        t.cover_url = cover;
        let value = activity(&t, u32::MAX, 1000);
        assert_eq!(value["timestamps"], json!({"start":820,"end":1000}));
        assert_eq!(value["assets"]["large_image"], "openify");
    }
    t.name = "\u{1f3b5}".repeat(100);
    assert_eq!(
        activity(&t, 0, 1000)["details"].as_str().unwrap().len(),
        128
    );
}

#[test]
fn private_paused_stopped_and_disabled_never_publish_metadata() {
    for (enabled, private, playing) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        assert!(desired(enabled, private, playing, Some(&track()), 0).is_none());
    }
    assert!(desired(true, false, true, None, 0).is_none());
    assert!(desired(true, false, true, Some(&track()), 0).is_some());
}

#[test]
fn failures_cannot_retry_until_fifteen_seconds_even_after_updates() {
    let now = Instant::now();
    let mut retry = Backoff::new(now);
    assert!(retry.ready(now));
    retry.failed(now);
    assert!(!retry.ready(now + Duration::from_millis(14_999)));
    assert!(retry.ready(now + RETRY));
    retry.failed(now + RETRY);
    assert!(!retry.ready(now + RETRY + Duration::from_secs(1)));
    assert!(retry.ready(now + RETRY * 2));
}

#[tokio::test]
async fn handshake_ping_nonce_fallback_and_clear_over_memory_transport() {
    let (client, server) = tokio::io::duplex(16384);
    let peer = tokio::spawn(async move {
        let mut peer = Connection::new(server);
        let (op, body) = peer.receive().await.unwrap();
        assert_eq!(op, 0);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"v":1,"client_id":"test-id"})
        );
        peer.send(3, b"ping").await.unwrap();
        assert_eq!(peer.receive().await.unwrap(), (4, b"ping".to_vec()));
        peer.send(1, br#"{"evt":"READY"}"#).await.unwrap();
        for i in 0..4 {
            let (op, body) = peer.receive().await.unwrap();
            assert_eq!(op, 1);
            let request: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(request["cmd"], "SET_ACTIVITY");
            assert_eq!(request["args"]["pid"], std::process::id());
            match i {
                0 => assert_eq!(
                    request["args"]["activity"]["assets"]["large_image"],
                    "https://i.scdn.co/image/test"
                ),
                1 | 2 => assert_eq!(
                    request["args"]["activity"]["assets"]["large_image"],
                    "openify"
                ),
                _ => assert!(request["args"]["activity"].is_null()),
            }
            peer.send(1, br#"{"nonce":"old","evt":"ERROR"}"#)
                .await
                .unwrap();
            peer.send(1,&serde_json::to_vec(&json!({"nonce":request["nonce"],"evt":if i == 0 {json!("ERROR")} else {Value::Null}})).unwrap()).await.unwrap();
        }
    });
    let mut connection = Connection::new(client);
    deadline(connection.handshake("test-id")).await.unwrap();
    deadline(publish(&mut connection, Some(activity(&track(), 0, 1000))))
        .await
        .unwrap();
    deadline(publish(
        &mut connection,
        Some(activity(&track(), 42_000, 1000)),
    ))
    .await
    .unwrap();
    deadline(publish(&mut connection, None)).await.unwrap();
    peer.await.unwrap();
}

#[tokio::test]
async fn closed_transport_and_invalid_handshake_fail_silently() {
    let (client, server) = tokio::io::duplex(1024);
    drop(server);
    assert!(
        deadline(Connection::new(client).handshake("test"))
            .await
            .is_err()
    );
    let (client, mut server) = tokio::io::duplex(1024);
    server.write_all(&encode_frame(2, b"{}")).await.unwrap();
    assert!(
        deadline(Connection::new(client).handshake("test"))
            .await
            .is_err()
    );
}

#[test]
fn settings_upgrade_defaults_off_and_roundtrips_opt_in() {
    let old: crate::settings::Settings = serde_json::from_str("{}").unwrap();
    assert!(!old.discord_presence);
    let enabled = crate::settings::Settings {
        discord_presence: true,
        ..old
    };
    let restored: crate::settings::Settings =
        serde_json::from_value(serde_json::to_value(enabled).unwrap()).unwrap();
    assert!(restored.discord_presence);
}

#[tokio::test]
async fn worker_clears_on_private_session_without_waiting_for_acknowledgement() {
    let (client, server) = tokio::io::duplex(16384);
    let (sender, receiver) = watch::channel(Desired {
        activity: Some(activity(&track(), 0, 1000)),
        shutdown: false,
    });
    let mut client = Some(client);
    let worker = tokio::spawn(worker_with(receiver, move || {
        let stream = client.take();
        async move {
            stream
                .map(Connection::new)
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
    }));
    let mut peer = Connection::new(server);
    let (_, body) = deadline(peer.receive()).await.unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(request["args"]["activity"]["details"], "Test song");
    peer.send(
        1,
        &serde_json::to_vec(&json!({"nonce":request["nonce"]})).unwrap(),
    )
    .await
    .unwrap();
    peer.send(3, b"sync").await.unwrap();
    assert_eq!(
        deadline(peer.receive()).await.unwrap(),
        (4, b"sync".to_vec())
    );
    sender.send_replace(Desired::default());
    let (_, body) = deadline(peer.receive()).await.unwrap();
    let clear: Value = serde_json::from_slice(&body).unwrap();
    assert!(clear["args"]["activity"].is_null());
    // The worker closes the stream even when Discord never acknowledges clear.
    assert!(deadline(peer.receive()).await.is_err());
    sender.send_replace(Desired {
        shutdown: true,
        activity: None,
    });
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn privacy_cancels_a_stalled_connection_without_publishing_old_track() {
    let (sender, receiver) = watch::channel(Desired {
        activity: Some(activity(&track(), 0, 1000)),
        shutdown: false,
    });
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut started = Some(started_tx);
    let worker = tokio::spawn(worker_with(receiver, move || {
        if let Some(started) = started.take() {
            let _ = started.send(());
        }
        std::future::pending::<io::Result<Connection<tokio::io::DuplexStream>>>()
    }));
    started_rx.await.unwrap();
    sender.send_replace(Desired::default());
    tokio::task::yield_now().await;
    sender.send_replace(Desired {
        shutdown: true,
        activity: None,
    });
    tokio::time::timeout(Duration::from_millis(100), worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn stalled_publication_is_cancelled_on_shutdown_and_stream_is_closed() {
    let (client, server) = tokio::io::duplex(16384);
    let (sender, receiver) = watch::channel(Desired {
        activity: Some(activity(&track(), 0, 1000)),
        shutdown: false,
    });
    let mut client = Some(client);
    let worker = tokio::spawn(worker_with(receiver, move || {
        let stream = client.take();
        async move {
            stream
                .map(Connection::new)
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
    }));
    let mut peer = Connection::new(server);
    deadline(peer.receive()).await.unwrap();
    sender.send_replace(Desired {
        shutdown: true,
        activity: None,
    });
    tokio::time::timeout(Duration::from_millis(100), worker)
        .await
        .unwrap()
        .unwrap();
    assert!(deadline(peer.receive()).await.is_err());
}

#[tokio::test]
async fn worker_does_not_retry_missing_transport_on_playback_updates() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut started = Some(started_tx);
    let (sender, receiver) = watch::channel(Desired {
        activity: Some(activity(&track(), 0, 1000)),
        shutdown: false,
    });
    let worker = tokio::spawn(worker_with(receiver, move || {
        count.fetch_add(1, Ordering::SeqCst);
        if let Some(started) = started.take() {
            let _ = started.send(());
        }
        std::future::ready(Err::<Connection<tokio::io::DuplexStream>, _>(
            io::ErrorKind::NotFound.into(),
        ))
    }));
    started_rx.await.unwrap();
    for position in [1000, 2000, 3000] {
        sender.send_replace(Desired {
            activity: Some(activity(&track(), position, 1000)),
            shutdown: false,
        });
        tokio::task::yield_now().await;
    }
    sender.send_replace(Desired {
        shutdown: true,
        activity: None,
    });
    worker.await.unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn closing_the_channel_cancels_pending_io_and_terminates_worker() {
    let (sender, receiver) = watch::channel(Desired {
        activity: Some(activity(&track(), 0, 1000)),
        shutdown: false,
    });
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut started = Some(started_tx);
    let worker = tokio::spawn(worker_with(receiver, move || {
        if let Some(started) = started.take() {
            let _ = started.send(());
        }
        std::future::pending::<io::Result<Connection<tokio::io::DuplexStream>>>()
    }));
    started_rx.await.unwrap();
    drop(sender);
    tokio::time::timeout(Duration::from_millis(100), worker)
        .await
        .unwrap()
        .unwrap();
}
