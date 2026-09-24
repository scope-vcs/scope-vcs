use crate::GitStorageError;
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, AeadInOut, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt};

pub const ENCODING_VERSION: u32 = 2;
const MAGIC: [u8; 8] = magic(ENCODING_VERSION);
const TAG_BYTES: usize = 16;
const FINAL_FLAG: u8 = 1;
const HEADER_FIXED_BYTES: usize = MAGIC.len() + 4 + 2 + 8 + 4;
const FRAME_HEADER_BYTES: usize = 4 + 4 + 1;
const MAX_KEY_ID_BYTES: usize = 1024;
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Whether `prefix`, the first bytes of a stored object, starts a framed envelope.
#[cfg(test)]
pub(crate) fn is_framed(prefix: &[u8]) -> bool {
    prefix.starts_with(&MAGIC)
}

const fn magic(version: u32) -> [u8; 8] {
    assert!(version < 100, "envelope magic encodes a two-digit version");
    let mut magic = *b"SCGSEG00";
    magic[6] = b'0' + (version / 10) as u8;
    magic[7] = b'0' + (version % 10) as u8;
    magic
}

/// What an envelope's key and authentication are bound to. Each scope derives its own cipher from
/// the storage key, so an object can only be read back under the identity it was written with.
#[derive(Clone, Debug)]
pub(crate) struct EnvelopeScope {
    label: &'static [u8],
    parts: Vec<String>,
}

impl EnvelopeScope {
    pub(crate) fn git_segment(repository_id: &str, segment_id: &str) -> Self {
        Self {
            label: b"scope-git-segment-v2\0",
            parts: vec![repository_id.to_string(), segment_id.to_string()],
        }
    }

    pub(crate) fn object(key: &str) -> Self {
        Self {
            label: b"scope-object-v2\0",
            parts: vec![key.to_string()],
        }
    }
}

#[derive(Clone)]
pub struct EncryptionKey {
    key_id: String,
    key: [u8; 32],
}

impl EncryptionKey {
    pub fn new(key_id: impl Into<String>, key: [u8; 32]) -> Result<Self, GitStorageError> {
        let key_id = key_id.into();
        if key_id.is_empty() || key_id.len() > MAX_KEY_ID_BYTES {
            return Err(GitStorageError::InvalidConfiguration(format!(
                "encryption key id must contain 1 to {MAX_KEY_ID_BYTES} bytes"
            )));
        }
        Ok(Self { key_id, key })
    }
}

pub(crate) struct EnvelopeWriter {
    cipher: ChaCha20Poly1305,
    header: Vec<u8>,
    nonce_prefix: [u8; 8],
    scope: EnvelopeScope,
    next_counter: u32,
}

impl EnvelopeWriter {
    pub(crate) fn new(
        key: &EncryptionKey,
        scope: EnvelopeScope,
        frame_bytes: usize,
    ) -> Result<Self, GitStorageError> {
        let frame_bytes = u32::try_from(frame_bytes).map_err(|_| {
            GitStorageError::InvalidConfiguration("encryption frame size exceeds u32".into())
        })?;
        let key_id_bytes = key.key_id.as_bytes();
        let key_id_len = u16::try_from(key_id_bytes.len()).map_err(|_| {
            GitStorageError::InvalidConfiguration("encryption key id is too long".into())
        })?;
        let mut nonce_prefix = [0_u8; 8];
        getrandom::fill(&mut nonce_prefix).map_err(|error| {
            GitStorageError::InvalidConfiguration(format!("creating segment nonce: {error}"))
        })?;
        let mut header = Vec::with_capacity(HEADER_FIXED_BYTES + key_id_bytes.len());
        header.extend_from_slice(&MAGIC);
        header.extend_from_slice(&ENCODING_VERSION.to_be_bytes());
        header.extend_from_slice(&key_id_len.to_be_bytes());
        header.extend_from_slice(&nonce_prefix);
        header.extend_from_slice(&frame_bytes.to_be_bytes());
        header.extend_from_slice(key_id_bytes);
        Ok(Self {
            cipher: scope_cipher(key, &scope),
            header,
            nonce_prefix,
            scope,
            next_counter: 0,
        })
    }

    pub(crate) fn header(&self) -> &[u8] {
        &self.header
    }

    pub(crate) fn encrypt_data(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, GitStorageError> {
        if plaintext.is_empty() {
            return Err(GitStorageError::InvalidEnvelope(
                "data frames cannot be empty".into(),
            ));
        }
        self.encrypt_frame(plaintext, 0)
    }

    pub(crate) fn encrypt_final(&mut self) -> Result<Vec<u8>, GitStorageError> {
        self.encrypt_frame(&[], FINAL_FLAG)
    }

    fn encrypt_frame(&mut self, plaintext: &[u8], flags: u8) -> Result<Vec<u8>, GitStorageError> {
        let mut frame = vec![0_u8; FRAME_HEADER_BYTES + plaintext.len() + TAG_BYTES];
        frame[FRAME_HEADER_BYTES..FRAME_HEADER_BYTES + plaintext.len()].copy_from_slice(plaintext);
        self.seal_frame(&mut frame, flags)?;
        Ok(frame)
    }

    /// Seals a frame laid out as header space, plaintext, and tag space, in place.
    fn seal_frame(&mut self, frame: &mut [u8], flags: u8) -> Result<(), GitStorageError> {
        let counter = self.next_counter;
        self.next_counter = self
            .next_counter
            .checked_add(1)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("too many encryption frames".into()))?;
        let tag_start = frame.len() - TAG_BYTES;
        let plaintext_len = u32::try_from(tag_start - FRAME_HEADER_BYTES)
            .map_err(|_| GitStorageError::InvalidEnvelope("encryption frame exceeds u32".into()))?;
        let frame_header = frame_header(counter, plaintext_len, flags);
        let aad = associated_data(&self.header, &self.scope, &frame_header);
        let nonce = nonce(self.nonce_prefix, counter);
        let tag = self
            .cipher
            .encrypt_inout_detached(
                &Nonce::from(nonce),
                &aad,
                (&mut frame[FRAME_HEADER_BYTES..tag_start]).into(),
            )
            .map_err(|_| GitStorageError::Encryption)?;
        frame[..FRAME_HEADER_BYTES].copy_from_slice(&frame_header);
        frame[tag_start..].copy_from_slice(&tag);
        Ok(())
    }
}

/// Seals `plaintext` as one complete envelope inside its own allocation, so a write never holds
/// the plaintext and a second copy of the envelope at once. Frames are moved back to front into
/// their sealed positions, which leaves every not-yet-moved frame intact.
pub(crate) fn seal(
    key: &EncryptionKey,
    scope: EnvelopeScope,
    frame_bytes: usize,
    mut buffer: Vec<u8>,
) -> Result<Vec<u8>, GitStorageError> {
    let mut writer = EnvelopeWriter::new(key, scope, frame_bytes)?;
    let header_len = writer.header.len();
    let plaintext_len = buffer.len();
    let frames = plaintext_len.div_ceil(frame_bytes);
    let overhead = FRAME_HEADER_BYTES + TAG_BYTES;
    let frame_start = |index: usize| header_len + index * (frame_bytes + overhead);
    let frame_len = |index: usize| frame_bytes.min(plaintext_len - index * frame_bytes);
    buffer.resize(header_len + frames * overhead + plaintext_len + overhead, 0);
    for index in (0..frames).rev() {
        let source = index * frame_bytes;
        buffer.copy_within(
            source..source + frame_len(index),
            frame_start(index) + FRAME_HEADER_BYTES,
        );
    }
    buffer[..header_len].copy_from_slice(&writer.header);
    for index in 0..frames {
        let start = frame_start(index);
        writer.seal_frame(&mut buffer[start..start + frame_len(index) + overhead], 0)?;
    }
    let final_start = buffer.len() - overhead;
    writer.seal_frame(&mut buffer[final_start..], FINAL_FLAG)?;
    Ok(buffer)
}

pub(crate) struct EnvelopeReader {
    cipher: ChaCha20Poly1305,
    header: Vec<u8>,
    nonce_prefix: [u8; 8],
    scope: EnvelopeScope,
    frame_bytes: usize,
    next_counter: u32,
    saw_final: bool,
}

pub(crate) enum DecryptedFrame {
    Data(Vec<u8>),
    Final,
}

impl EnvelopeReader {
    pub(crate) async fn read_header<R: AsyncRead + Unpin>(
        source: &mut R,
        key: &EncryptionKey,
        scope: EnvelopeScope,
    ) -> Result<Self, GitStorageError> {
        let mut fixed = [0_u8; HEADER_FIXED_BYTES];
        read_exact_envelope(source, &mut fixed).await?;
        if fixed[..MAGIC.len()] != MAGIC {
            return Err(GitStorageError::InvalidEnvelope("wrong magic".into()));
        }
        let version = u32::from_be_bytes(fixed[8..12].try_into().expect("fixed slice"));
        if version != ENCODING_VERSION {
            return Err(GitStorageError::InvalidEnvelope(format!(
                "unsupported encoding version {version}"
            )));
        }
        let key_id_len =
            u16::from_be_bytes(fixed[12..14].try_into().expect("fixed slice")) as usize;
        if key_id_len == 0 || key_id_len > MAX_KEY_ID_BYTES {
            return Err(GitStorageError::InvalidEnvelope(
                "invalid encryption key id length".into(),
            ));
        }
        let nonce_prefix = fixed[14..22].try_into().expect("fixed slice");
        let frame_bytes =
            u32::from_be_bytes(fixed[22..26].try_into().expect("fixed slice")) as usize;
        if frame_bytes == 0 || frame_bytes > MAX_FRAME_BYTES {
            return Err(GitStorageError::InvalidEnvelope(
                "invalid encryption frame size".into(),
            ));
        }
        let mut key_id = vec![0_u8; key_id_len];
        read_exact_envelope(source, &mut key_id).await?;
        if key_id != key.key_id.as_bytes() {
            return Err(GitStorageError::InvalidEnvelope(
                "encryption key id does not match".into(),
            ));
        }
        let mut header = fixed.to_vec();
        header.extend_from_slice(&key_id);
        Ok(Self {
            cipher: scope_cipher(key, &scope),
            header,
            nonce_prefix,
            scope,
            frame_bytes,
            next_counter: 0,
            saw_final: false,
        })
    }

    pub(crate) async fn next<R: AsyncRead + Unpin>(
        &mut self,
        source: &mut R,
    ) -> Result<DecryptedFrame, GitStorageError> {
        if self.saw_final {
            return Err(GitStorageError::InvalidEnvelope(
                "data follows the final frame".into(),
            ));
        }
        let mut frame_header_bytes = [0_u8; FRAME_HEADER_BYTES];
        read_exact_envelope(source, &mut frame_header_bytes).await?;
        let counter = u32::from_be_bytes(frame_header_bytes[..4].try_into().expect("fixed slice"));
        let plaintext_len =
            u32::from_be_bytes(frame_header_bytes[4..8].try_into().expect("fixed slice")) as usize;
        let flags = frame_header_bytes[8];
        if counter != self.next_counter {
            return Err(GitStorageError::InvalidEnvelope(format!(
                "frame {counter} arrived where frame {} was required",
                self.next_counter
            )));
        }
        if flags & !FINAL_FLAG != 0 {
            return Err(GitStorageError::InvalidEnvelope(
                "frame has unknown flags".into(),
            ));
        }
        let is_final = flags == FINAL_FLAG;
        if is_final != (plaintext_len == 0) {
            return Err(GitStorageError::InvalidEnvelope(
                "only the final frame may be empty".into(),
            ));
        }
        if plaintext_len > self.frame_bytes {
            return Err(GitStorageError::InvalidEnvelope(
                "frame exceeds the envelope frame size".into(),
            ));
        }
        let mut ciphertext = vec![0_u8; plaintext_len + TAG_BYTES];
        read_exact_envelope(source, &mut ciphertext).await?;
        let aad = associated_data(&self.header, &self.scope, &frame_header_bytes);
        let nonce = nonce(self.nonce_prefix, counter);
        let plaintext = self
            .cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| GitStorageError::InvalidEnvelope("frame authentication failed".into()))?;
        self.next_counter = self
            .next_counter
            .checked_add(1)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("too many encryption frames".into()))?;
        if is_final {
            self.saw_final = true;
            Ok(DecryptedFrame::Final)
        } else {
            Ok(DecryptedFrame::Data(plaintext))
        }
    }
}

fn frame_header(counter: u32, plaintext_len: u32, flags: u8) -> [u8; FRAME_HEADER_BYTES] {
    let mut header = [0_u8; FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(&counter.to_be_bytes());
    header[4..8].copy_from_slice(&plaintext_len.to_be_bytes());
    header[8] = flags;
    header
}

fn scope_cipher(key: &EncryptionKey, scope: &EnvelopeScope) -> ChaCha20Poly1305 {
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(&key.key)
        .expect("HMAC accepts a 32-byte key");
    mac.update(scope.label);
    for part in &scope.parts {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part.as_bytes());
    }
    let derived_key = mac.finalize().into_bytes();
    ChaCha20Poly1305::new_from_slice(&derived_key).expect("HMAC derives a 32-byte key")
}

fn nonce(prefix: [u8; 8], counter: u32) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[..8].copy_from_slice(&prefix);
    nonce[8..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

fn associated_data(
    header: &[u8],
    scope: &EnvelopeScope,
    frame_header: &[u8; FRAME_HEADER_BYTES],
) -> Vec<u8> {
    let mut aad = header.to_vec();
    for part in &scope.parts {
        aad.extend_from_slice(&(part.len() as u32).to_be_bytes());
        aad.extend_from_slice(part.as_bytes());
    }
    aad.extend_from_slice(frame_header);
    aad
}

async fn read_exact_envelope<R: AsyncRead + Unpin>(
    source: &mut R,
    target: &mut [u8],
) -> Result<(), GitStorageError> {
    source
        .read_exact(target)
        .await
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                GitStorageError::InvalidEnvelope("truncated stream".into())
            } else {
                GitStorageError::Backend(crate::BackendError::new(error.to_string()))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_python_recovery_segment() {
        // Produced by deploy/aws/recovery/tests/test_recovery.py::segment.
        let fixture = hex::decode("53434753454730320000000200076e6e6e6e6e6e6e6e000004007072696d6172790000000000000012003b36a869a52a9b5f9c5f06ab0beec0d365170a7c5807b055345ccc7a7c302189b8df000000010000000001e3030b235680d3a18f50e0d2e52c20f9").unwrap();
        let key = EncryptionKey::new("primary", [b'k'; 32]).unwrap();
        let mut source = fixture.as_slice();
        let mut reader = EnvelopeReader::read_header(
            &mut source,
            &key,
            EnvelopeScope::git_segment("repository-123", "segment-123"),
        )
        .await
        .unwrap();
        let DecryptedFrame::Data(bytes) = reader.next(&mut source).await.unwrap() else {
            panic!("expected a data frame");
        };
        assert_eq!(bytes, b"ciphertext fixture");
        assert!(matches!(
            reader.next(&mut source).await.unwrap(),
            DecryptedFrame::Final
        ));
        assert!(source.is_empty());
    }

    /// Seals one data frame exactly as the segment-only envelope did before objects shared it.
    fn segment_sealed_by_the_original_derivation(
        key: &[u8; 32],
        repository_id: &str,
        segment_id: &str,
        plaintext: &[u8],
    ) -> Vec<u8> {
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key).unwrap();
        mac.update(b"scope-git-segment-v2\0");
        mac.update(&(repository_id.len() as u64).to_be_bytes());
        mac.update(repository_id.as_bytes());
        mac.update(&(segment_id.len() as u64).to_be_bytes());
        mac.update(segment_id.as_bytes());
        let cipher = ChaCha20Poly1305::new_from_slice(&mac.finalize().into_bytes()).unwrap();
        let nonce_prefix = [9_u8; 8];
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(&ENCODING_VERSION.to_be_bytes());
        header.extend_from_slice(&7_u16.to_be_bytes());
        header.extend_from_slice(&nonce_prefix);
        header.extend_from_slice(&1024_u32.to_be_bytes());
        header.extend_from_slice(b"primary");
        let mut sealed = header.clone();
        for (counter, (frame, flags)) in [(plaintext, 0), (&[][..], FINAL_FLAG)]
            .into_iter()
            .enumerate()
        {
            let frame_header = frame_header(counter as u32, frame.len() as u32, flags);
            let mut aad = header.clone();
            aad.extend_from_slice(&(repository_id.len() as u32).to_be_bytes());
            aad.extend_from_slice(repository_id.as_bytes());
            aad.extend_from_slice(&(segment_id.len() as u32).to_be_bytes());
            aad.extend_from_slice(segment_id.as_bytes());
            aad.extend_from_slice(&frame_header);
            let ciphertext = cipher
                .encrypt(
                    &Nonce::from(nonce(nonce_prefix, counter as u32)),
                    Payload {
                        msg: frame,
                        aad: &aad,
                    },
                )
                .unwrap();
            sealed.extend_from_slice(&frame_header);
            sealed.extend_from_slice(&ciphertext);
        }
        sealed
    }

    #[tokio::test]
    async fn segments_sealed_before_objects_shared_the_envelope_still_open() {
        let key = EncryptionKey::new("primary", [5_u8; 32]).unwrap();
        let sealed =
            segment_sealed_by_the_original_derivation(&[5_u8; 32], "repo", "segment", b"pack");
        let mut source = &sealed[..];
        let mut reader = EnvelopeReader::read_header(
            &mut source,
            &key,
            EnvelopeScope::git_segment("repo", "segment"),
        )
        .await
        .unwrap();
        let DecryptedFrame::Data(frame) = reader.next(&mut source).await.unwrap() else {
            panic!("expected a data frame");
        };
        assert_eq!(frame, b"pack");
        assert!(matches!(
            reader.next(&mut source).await.unwrap(),
            DecryptedFrame::Final
        ));

        // The same bytes do not open as an object, whose key is derived under its own label.
        let mut source = &sealed[..];
        let mut object =
            EnvelopeReader::read_header(&mut source, &key, EnvelopeScope::object("repo"))
                .await
                .unwrap();
        assert!(object.next(&mut source).await.is_err());
    }
}
