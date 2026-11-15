//! SHA-256 computed while a file is written, so a download is never read back to be verified.

use std::path::Path;

use lampo_domain::package::Sha256Digest;
use lampo_domain::{PortError, PortResult};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

/// Read buffer used to hash the part of a download that is already on disk.
const PREFIX_BUFFER_BYTES: usize = 1 << 20;

/// SHA-256 of the bytes fed so far, in order, and how many they are.
#[derive(Default)]
pub(crate) struct IncrementalSha256 {
    hasher: Sha256,
    len: u64,
}

impl IncrementalSha256 {
    /// The state after the first `len` bytes of `file` (a partial download being resumed):
    /// those bytes are read once, the rest is hashed as it arrives. `InvalidData` if the file is
    /// shorter than `len`.
    pub(crate) async fn of_prefix(file: &Path, len: u64) -> PortResult<Self> {
        let io = |e: std::io::Error| PortError::io(format!("hashing {}", file.display()), e);
        let mut reader = tokio::fs::File::open(file).await.map_err(io)?.take(len);
        let mut state = Self::default();
        let mut buf = vec![0u8; PREFIX_BUFFER_BYTES];
        loop {
            let n = reader.read(&mut buf).await.map_err(io)?;
            if n == 0 {
                break;
            }
            state.update(&buf[..n]);
        }
        if state.len != len {
            return Err(PortError::invalid_data(file.display().to_string(), format!("expected {len} bytes to resume from, found {}", state.len)));
        }
        Ok(state)
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
        self.len += bytes.len() as u64;
    }

    /// Bytes hashed so far.
    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    pub(crate) fn finish(self) -> Sha256Digest {
        Sha256Digest::from_bytes(self.hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn hashes_known_content_fed_in_pieces() {
        assert_eq!(IncrementalSha256::default().finish().to_string(), EMPTY);
        let mut state = IncrementalSha256::default();
        state.update(b"a");
        state.update(b"bc");
        assert_eq!(state.len(), 3);
        assert_eq!(state.finish().to_string(), ABC);
    }

    #[tokio::test]
    async fn resumes_from_the_bytes_already_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        std::fs::write(&part, b"ab").unwrap();
        let mut state = IncrementalSha256::of_prefix(&part, 2).await.unwrap();
        state.update(b"c");
        assert_eq!(state.finish().to_string(), ABC);
    }

    #[tokio::test]
    async fn a_prefix_longer_than_the_file_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        std::fs::write(&part, b"ab").unwrap();
        let err = IncrementalSha256::of_prefix(&part, 5).await.err().unwrap();
        assert!(matches!(err, PortError::InvalidData { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn missing_file_is_an_io_error() {
        let err = IncrementalSha256::of_prefix(Path::new("Z:\\definitely\\missing.zip"), 1).await.err().unwrap();
        assert!(matches!(err, PortError::Io { .. }));
    }
}
