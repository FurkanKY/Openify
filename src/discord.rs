//! Optional Discord Rich Presence. Only the worker owns IPC and its runtime.
//! A watch channel coalesces updates; old song metadata is never queued.

use crate::engine::TrackInfo;
use serde_json::{Value, json};
use std::{
    io,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch,
};

/// Fill in the application ID from the Discord developer portal.
pub const DISCORD_CLIENT_ID: &str = "";
const RETRY: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_FRAME: usize = 64 * 1024;
pub fn available() -> bool {
    !DISCORD_CLIENT_ID.trim().is_empty()
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid Discord IPC frame")
}

fn encode_frame(opcode: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + body.len());
    bytes.extend_from_slice(&opcode.to_le_bytes());
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes
}

/// Incomplete frames stay buffered, including across cancelled reads.
fn decode_frame(input: &mut Vec<u8>) -> io::Result<Option<(u32, Vec<u8>)>> {
    if input.len() < 8 {
        return Ok(None);
    }
    let opcode = u32::from_le_bytes(input[..4].try_into().unwrap());
    let len = u32::from_le_bytes(input[4..8].try_into().unwrap()) as usize;
    if opcode > 4 || len > MAX_FRAME {
        return Err(invalid());
    }
    if input.len() < 8 + len {
        return Ok(None);
    }
    let body = input[8..8 + len].to_vec();
    input.drain(..8 + len);
    Ok(Some((opcode, body)))
}

fn short(text: &str) -> String {
    let mut end = text.len().min(128);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub fn activity(track: &TrackInfo, position_ms: u32, now: u64) -> Value {
    let position = position_ms.min(track.duration_ms) as u64 / 1000;
    let start = now.saturating_sub(position);
    let image = track
        .cover_url
        .as_deref()
        .filter(|url| url.starts_with("https://i.scdn.co/") && url.len() <= 1024)
        .unwrap_or("openify");
    json!({"type":2,"status_display_type":2,"details":short(&track.name),"state":short(&track.artists),
        "timestamps":{"start":start,"end":start + u64::from(track.duration_ms)/1000},
        "assets":{"large_image":image,"large_text":short(&track.name)}})
}

pub fn desired(
    enabled: bool,
    private: bool,
    playing: bool,
    track: Option<&TrackInfo>,
    position_ms: u32,
) -> Option<Value> {
    if !enabled || private || !playing {
        return None;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    track.map(|track| activity(track, position_ms, now))
}

#[derive(Clone, Default, PartialEq)]
struct Desired {
    activity: Option<Value>,
    shutdown: bool,
}
struct Backoff {
    next: Instant,
}
impl Backoff {
    fn new(now: Instant) -> Self {
        Self { next: now }
    }
    fn ready(&self, now: Instant) -> bool {
        now >= self.next
    }
    fn failed(&mut self, now: Instant) {
        self.next = now + RETRY;
    }
}

/// Any asynchronous byte stream is a transport; tests use an in-memory peer.
struct Connection<S> {
    stream: S,
    input: Vec<u8>,
    nonce: u64,
    external_images: bool,
}
impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    fn new(stream: S) -> Self {
        Self {
            stream,
            input: Vec::new(),
            nonce: 0,
            external_images: true,
        }
    }
    async fn send(&mut self, opcode: u32, body: &[u8]) -> io::Result<()> {
        self.stream.write_all(&encode_frame(opcode, body)).await
    }
    async fn receive(&mut self) -> io::Result<(u32, Vec<u8>)> {
        loop {
            if let Some(frame) = decode_frame(&mut self.input)? {
                return Ok(frame);
            }
            if self.stream.read_buf(&mut self.input).await? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }
    async fn message(&mut self) -> io::Result<Value> {
        loop {
            let (op, body) = self.receive().await?;
            match op {
                1 => return serde_json::from_slice(&body).map_err(|_| invalid()),
                3 => self.send(4, &body).await?,
                4 => {}
                _ => return Err(invalid()),
            }
        }
    }
    async fn handshake(&mut self, client_id: &str) -> io::Result<()> {
        self.send(
            0,
            &serde_json::to_vec(&json!({"v":1,"client_id":client_id}))?,
        )
        .await?;
        if self.message().await?["evt"] != "READY" {
            return Err(invalid());
        }
        Ok(())
    }
    async fn publish(&mut self, activity: Option<Value>) -> io::Result<bool> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        let body = json!({"cmd":"SET_ACTIVITY","args":{"pid":std::process::id(),"activity":activity},"nonce":nonce});
        self.send(1, &serde_json::to_vec(&body)?).await?;
        loop {
            let reply = self.message().await?;
            if reply["nonce"] == nonce {
                return Ok(reply["evt"] != "ERROR");
            }
        }
    }

    async fn clear(&mut self) -> io::Result<()> {
        self.nonce += 1;
        let body = json!({"cmd":"SET_ACTIVITY","args":{"pid":std::process::id(),"activity":null},"nonce":self.nonce.to_string()});
        self.send(1, &serde_json::to_vec(&body)?).await
    }
}

async fn publish<S: AsyncRead + AsyncWrite + Unpin>(
    connection: &mut Connection<S>,
    mut value: Option<Value>,
) -> io::Result<()> {
    if !connection.external_images
        && let Some(value) = &mut value
    {
        value["assets"]["large_image"] = json!("openify");
    }
    if connection.publish(value.clone()).await? {
        return Ok(());
    }
    if let Some(mut fallback) = value
        && fallback["assets"]["large_image"]
            .as_str()
            .is_some_and(|s| s.starts_with("https://"))
    {
        fallback["assets"]["large_image"] = json!("openify");
        if connection.publish(Some(fallback)).await? {
            connection.external_images = false;
            return Ok(());
        }
    }
    Err(invalid())
}

pub struct Presence {
    sender: Option<watch::Sender<Desired>>,
    done: Option<async_channel::Receiver<()>>,
}
impl Presence {
    pub fn new() -> Self {
        // Test copies never contact the user's running Discord.
        if !available() || crate::system::test_mode() {
            return Self {
                sender: None,
                done: None,
            };
        }
        let (sender, receiver) = watch::channel(Desired::default());
        let (done_tx, done) = async_channel::bounded(1);
        if std::thread::Builder::new()
            .name("openify-discord".into())
            .spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(worker(receiver));
                }
                let _ = done_tx.try_send(());
            })
            .is_err()
        {
            return Self {
                sender: None,
                done: None,
            };
        }
        Self {
            sender: Some(sender),
            done: Some(done),
        }
    }
    pub fn update(&self, activity: Option<Value>) {
        if let Some(sender) = &self.sender {
            sender.send_if_modified(|state| {
                if state.shutdown || state.activity == activity {
                    return false;
                }
                state.activity = activity;
                true
            });
        }
    }
    pub fn shutdown(&mut self) -> impl Future<Output = ()> + use<> {
        if let Some(sender) = self.sender.take() {
            sender.send_replace(Desired {
                shutdown: true,
                activity: None,
            });
        }
        let done = self.done.take();
        async move {
            if let Some(done) = done {
                let _ = done.recv().await;
            }
        }
    }
}
impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            sender.send_replace(Desired {
                shutdown: true,
                activity: None,
            });
        }
    }
}

async fn deadline<T>(future: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    tokio::time::timeout(IO_TIMEOUT, future)
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

/// Privacy and quit can cancel an in-flight handshake or update. The caller
/// drops the stream on cancellation, since a partial write cannot be reused.
async fn interruptible<T>(
    future: impl Future<Output = io::Result<T>>,
    desired: &mut watch::Receiver<Desired>,
) -> io::Result<T> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            changed = desired.changed() => {
                let state = desired.borrow();
                if changed.is_err() || state.shutdown || state.activity.is_none() {
                    return Err(io::ErrorKind::Interrupted.into());
                }
            },
            result = &mut future => return result,
        }
    }
}

async fn connect() -> io::Result<Connection<tokio::net::windows::named_pipe::NamedPipeClient>> {
    use tokio::net::windows::named_pipe::ClientOptions;
    for index in 0..10 {
        if let Ok(pipe) = ClientOptions::new().open(format!(r"\\.\pipe\discord-ipc-{index}")) {
            let mut connection = Connection::new(pipe);
            if deadline(connection.handshake(DISCORD_CLIENT_ID))
                .await
                .is_ok()
            {
                return Ok(connection);
            }
        }
    }
    Err(io::ErrorKind::NotFound.into())
}

async fn worker(desired: watch::Receiver<Desired>) {
    worker_with(desired, connect).await;
}

async fn worker_with<S, F, Fut>(mut desired: watch::Receiver<Desired>, mut connect: F)
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut() -> Fut,
    Fut: Future<Output = io::Result<Connection<S>>>,
{
    let mut connection: Option<Connection<S>> = None;
    let mut sent = None;
    let mut backoff = Backoff::new(Instant::now());
    let mut next_publish = Instant::now();
    loop {
        let state = desired.borrow_and_update().clone();
        let disconnected = desired.has_changed().is_err();
        if state.shutdown || disconnected || state.activity.is_none() {
            if let Some(mut pipe) = connection.take() {
                // gpui allows 100 ms for quit futures. Send the clear without
                // waiting for its acknowledgement, then close the pipe.
                let _ = tokio::time::timeout(Duration::from_millis(50), pipe.clear()).await;
            }
            sent = None;
            if state.shutdown || disconnected {
                break;
            }
            if desired.changed().await.is_err() {
                break;
            }
            continue;
        }
        if connection.is_none() && backoff.ready(Instant::now()) {
            match interruptible(connect(), &mut desired).await {
                Ok(pipe) => {
                    connection = Some(pipe);
                    sent = None;
                    continue;
                }
                Err(error) => {
                    if error.kind() != io::ErrorKind::Interrupted {
                        backoff.failed(Instant::now());
                    }
                    continue;
                }
            }
        }
        if let Some(pipe) = &mut connection {
            if state.activity != sent && Instant::now() >= next_publish {
                match interruptible(
                    deadline(publish(pipe, state.activity.clone())),
                    &mut desired,
                )
                .await
                {
                    Err(error) => {
                        connection = None;
                        if error.kind() != io::ErrorKind::Interrupted {
                            backoff.failed(Instant::now());
                        }
                    }
                    Ok(()) => {
                        sent = state.activity;
                        next_publish = Instant::now() + Duration::from_secs(5);
                    }
                }
                continue;
            }
            tokio::select! {
                changed = desired.changed() => { if changed.is_err() { break; } },
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                result = pipe.receive() => {
                    let result = match result {
                        Ok((3, body)) => interruptible(deadline(pipe.send(4, &body)), &mut desired).await,
                        Ok((1 | 4, _)) => Ok(()),
                        _ => Err(invalid()),
                    };
                    if let Err(error) = result {
                        connection = None;
                        if error.kind() != io::ErrorKind::Interrupted { backoff.failed(Instant::now()); }
                    }
                }
            }
        } else {
            tokio::select! {
                changed = desired.changed() => { if changed.is_err() { break; } },
                _ = tokio::time::sleep_until(backoff.next.into()) => {},
            }
        }
    }
}

#[cfg(test)]
mod tests;
