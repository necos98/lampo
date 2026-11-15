use std::io::Read;
use std::path::Path;

use async_trait::async_trait;
use lampo_domain::package::Sha256Digest;
use lampo_domain::ports::ChecksumCalculator;
use lampo_domain::{PortError, PortResult};
use sha2::{Digest, Sha256};

/// Streaming SHA-256 on a blocking thread.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sha256Calculator;

impl Sha256Calculator {
    /// Synchronous variant (used on blocking threads and in tests).
    pub fn sha256_blocking(file: &Path) -> PortResult<Sha256Digest> {
        let io = |e: std::io::Error| PortError::io(format!("hashing {}", file.display()), e);
        let mut f = std::fs::File::open(file).map_err(io)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf).map_err(io)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
    }
}

#[async_trait]
impl ChecksumCalculator for Sha256Calculator {
    async fn sha256(&self, file: &Path) -> PortResult<Sha256Digest> {
        let file = file.to_path_buf();
        tokio::task::spawn_blocking(move || Self::sha256_blocking(&file)).await.map_err(|e| PortError::io("hashing a file", e))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(Sha256Calculator.sha256(&empty).await.unwrap().to_string(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let abc = dir.path().join("abc");
        std::fs::write(&abc, b"abc").unwrap();
        assert_eq!(Sha256Calculator.sha256(&abc).await.unwrap().to_string(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[tokio::test]
    async fn missing_file_is_an_io_error() {
        let err = Sha256Calculator.sha256(Path::new("Z:\\definitely\\missing.zip")).await.unwrap_err();
        assert!(matches!(err, PortError::Io { .. }));
    }
}
