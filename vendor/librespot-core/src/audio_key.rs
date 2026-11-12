use std::{collections::HashMap, io::Write, time::Duration};

use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use bytes::Bytes;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::{Error, FileId, SpotifyId, packet::PacketType, util::SeqGenerator};

#[derive(Debug, Hash, PartialEq, Eq, Copy, Clone)]
pub struct AudioKey(pub [u8; 16]);

#[derive(Debug, Error)]
pub enum AudioKeyError {
    #[error("audio key error")]
    AesKey,
    #[error("other end of channel disconnected")]
    Channel,
    #[error("unexpected packet type {0}")]
    Packet(u8),
    #[error("sequence {0} not pending")]
    Sequence(u32),
    #[error("audio key response timeout")]
    Timeout,
    #[error("session closed while waiting for audio key")]
    SessionClosed,
}

impl From<AudioKeyError> for Error {
    fn from(err: AudioKeyError) -> Self {
        match err {
            AudioKeyError::AesKey => Error::unavailable(err),
            AudioKeyError::Channel => Error::aborted(err),
            AudioKeyError::Sequence(_) => Error::aborted(err),
            AudioKeyError::Packet(_) => Error::unimplemented(err),
            AudioKeyError::Timeout => Error::aborted(err),
            AudioKeyError::SessionClosed => Error::aborted(err),
        }
    }
}

component! {
    AudioKeyManager : AudioKeyManagerInner {
        sequence: SeqGenerator<u32> = SeqGenerator::new(0),
        pending: HashMap<u32, oneshot::Sender<Result<AudioKey, Error>>> = HashMap::new(),
        // Openify: a key, once received, is remembered for the session, so
        // replaying a track does not ask Spotify again.
        known: HashMap<(SpotifyId, FileId), AudioKey> = HashMap::new(),
    }
}

// Openify: Spotify throttles audio key requests under load, either by
// answering late (seen: ~3.5 s) or by refusing them (AesKeyError "0 2").
// Upstream asks once and gives up after 1.5 s, after which the player goes on
// without decryption and fails. Here a request waits longer and is retried
// with backoff.
const KEY_RESPONSE_TIMEOUT: Duration = Duration::from_millis(4000);
// Openify: while waiting, the session is polled this often; once it is
// invalidated (reconnect replaced it, or the connection was seen closing) the
// request fails at once instead of sitting out the remaining timeout.
const SESSION_POLL: Duration = Duration::from_millis(250);
const KEY_RETRY_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(1000),
    Duration::from_millis(2000),
];

enum Waited {
    Answer(Result<Result<AudioKey, Error>, oneshot::error::RecvError>),
    Timeout,
    SessionClosed,
}

impl AudioKeyManager {
    pub(crate) fn dispatch(&self, cmd: PacketType, mut data: Bytes) -> Result<(), Error> {
        let seq = BigEndian::read_u32(data.split_to(4).as_ref());

        let sender = self
            .lock(|inner| inner.pending.remove(&seq))
            .ok_or(AudioKeyError::Sequence(seq))?;

        match cmd {
            PacketType::AesKey => {
                let mut key = [0u8; 16];
                key.copy_from_slice(data.as_ref());
                sender
                    .send(Ok(AudioKey(key)))
                    .map_err(|_| AudioKeyError::Channel)?
            }
            PacketType::AesKeyError => {
                error!(
                    "error audio key {:x} {:x}",
                    data.as_ref()[0],
                    data.as_ref()[1]
                );
                sender
                    .send(Err(AudioKeyError::AesKey.into()))
                    .map_err(|_| AudioKeyError::Channel)?
            }
            _ => {
                trace!("Did not expect {cmd:?} AES key packet with data {data:#?}");
                return Err(AudioKeyError::Packet(cmd as u8).into());
            }
        }

        Ok(())
    }

    pub async fn request(&self, track: SpotifyId, file: FileId) -> Result<AudioKey, Error> {
        if let Some(key) = self.lock(|inner| inner.known.get(&(track, file)).copied()) {
            return Ok(key);
        }

        let mut last_error: Error = AudioKeyError::Timeout.into();
        for (attempt, delay) in KEY_RETRY_DELAYS.iter().enumerate() {
            if !delay.is_zero() {
                tokio::time::sleep(*delay).await;
            }
            // A dead session never answers: do not send, do not wait.
            if self.session().is_invalid() {
                warn!("Audio key request abandoned: session closed");
                return Err(AudioKeyError::SessionClosed.into());
            }

            let (tx, rx) = oneshot::channel();
            let seq = self.lock(move |inner| {
                let seq = inner.sequence.get();
                inner.pending.insert(seq, tx);
                seq
            });
            self.send_key_request(seq, track, file)?;

            match self.wait_response(rx).await {
                Waited::Answer(Ok(Ok(key))) => {
                    if attempt > 0 {
                        info!("Audio key received on attempt {}", attempt + 1);
                    }
                    self.lock(|inner| inner.known.insert((track, file), key));
                    return Ok(key);
                }
                Waited::Answer(Ok(Err(e))) => {
                    warn!("Audio key refused (attempt {}): {e}", attempt + 1);
                    last_error = e;
                }
                Waited::Answer(Err(_)) => {
                    last_error = AudioKeyError::Channel.into();
                }
                Waited::Timeout => {
                    // Forget the request so a late answer is not dispatched
                    // to a dropped receiver.
                    self.lock(|inner| inner.pending.remove(&seq));
                    warn!("Audio key response timeout (attempt {})", attempt + 1);
                    last_error = AudioKeyError::Timeout.into();
                }
                Waited::SessionClosed => {
                    self.lock(|inner| inner.pending.remove(&seq));
                    warn!("Audio key request abandoned: session closed");
                    return Err(AudioKeyError::SessionClosed.into());
                }
            }
        }

        error!("Audio key request failed after {} attempts", KEY_RETRY_DELAYS.len());
        Err(last_error)
    }

    /// Waits for the answer up to `KEY_RESPONSE_TIMEOUT`, giving up early when
    /// the session is invalidated meanwhile.
    async fn wait_response(
        &self,
        mut rx: oneshot::Receiver<Result<AudioKey, Error>>,
    ) -> Waited {
        let deadline = tokio::time::Instant::now() + KEY_RESPONSE_TIMEOUT;
        loop {
            let step = SESSION_POLL.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
            match tokio::time::timeout(step, &mut rx).await {
                Ok(answer) => return Waited::Answer(answer),
                Err(_) => {
                    if self.session().is_invalid() {
                        return Waited::SessionClosed;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Waited::Timeout;
                    }
                }
            }
        }
    }

    fn send_key_request(&self, seq: u32, track: SpotifyId, file: FileId) -> Result<(), Error> {
        let mut data: Vec<u8> = Vec::new();
        data.write_all(&file.0)?;
        data.write_all(&track.to_raw())?;
        data.write_u32::<BigEndian>(seq)?;
        data.write_u16::<BigEndian>(0x0000)?;

        self.session().send_packet(PacketType::RequestKey, data)
    }
}
