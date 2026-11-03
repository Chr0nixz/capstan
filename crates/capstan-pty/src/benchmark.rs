//! Opt-in, synthetic-data-only measurement of the real local PTY path.
use crate::{
    ConPtyConfig, ConPtySelection, PtyCommand, PtySession, PtySessionConfig, ShellInfo, ShellKind,
    ShellSource,
};
use capstan_terminal::BackendParts;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};

pub const BULK_BYTES: u64 = 100 * 1024 * 1024;
pub const HASH_SEED: u32 = 0x811c_9dc5;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Bulk,
    Idle,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadDigest {
    pub bytes: u64,
    pub checksum: u32,
    pub eof: bool,
}
impl Default for ReadDigest {
    fn default() -> Self {
        Self {
            bytes: 0,
            checksum: HASH_SEED,
            eof: false,
        }
    }
}
pub type SharedDigest = Arc<Mutex<ReadDigest>>;

pub fn measure_async_backend(
    mut backend: capstan_terminal::AsyncBackendParts,
) -> (capstan_terminal::AsyncBackendParts, SharedDigest) {
    let digest = Arc::new(Mutex::new(ReadDigest::default()));
    backend.reader = Box::pin(MeasuredAsyncReader {
        reader: backend.reader,
        digest: digest.clone(),
    });
    (backend, digest)
}

struct MeasuredAsyncReader {
    reader: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>,
    digest: SharedDigest,
}
impl tokio::io::AsyncRead for MeasuredAsyncReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let capacity = buffer.remaining();
        let result = self.reader.as_mut().poll_read(context, buffer);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let bytes = &buffer.filled()[before..];
            let mut digest = self
                .digest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if bytes.is_empty() && capacity > 0 {
                digest.eof = true;
            }
            for byte in bytes {
                digest.checksum = (digest.checksum ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
            }
            digest.bytes += bytes.len() as u64;
        }
        result
    }
}

struct MeasuredReader {
    reader: Box<dyn Read + Send>,
    digest: SharedDigest,
}
impl Read for MeasuredReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.reader.read(buffer)?;
        let mut digest = self
            .digest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if count == 0 {
            digest.eof = true;
        }
        for byte in &buffer[..count] {
            digest.checksum = (digest.checksum ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
        }
        digest.bytes += count as u64;
        Ok(count)
    }
}

pub fn open_fixture(
    executable: &Path,
    scenario: Scenario,
    runtime: &ConPtyConfig,
) -> crate::Result<(BackendParts, (ConPtySelection, SharedDigest))> {
    if !executable.is_absolute() || !executable.is_file() {
        return Err(crate::PtyError::Pty("benchmark_fixture_missing".to_owned()));
    }
    let args = match scenario {
        Scenario::Bulk => vec!["--bulk".to_owned()],
        Scenario::Idle => vec!["--idle".to_owned()],
    };
    let session = PtySession::spawn_with_runtime(
        &PtySessionConfig {
            command: PtyCommand {
                shell: ShellInfo {
                    kind: ShellKind::Custom,
                    executable: executable.to_owned(),
                    args,
                    display_name: "M0 fixture".to_owned(),
                    source: ShellSource::UserProvided,
                },
                working_directory: None,
                environment: Default::default(),
            },
            columns: 80,
            rows: 24,
        },
        runtime,
    )?;
    let selected = session.runtime().clone();
    let mut backend = session.into_backend()?;
    let digest = Arc::new(Mutex::new(ReadDigest::default()));
    backend.reader = Box::new(MeasuredReader {
        reader: backend.reader,
        digest: digest.clone(),
    });
    Ok((backend, (selected, digest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    #[tokio::test]
    async fn async_digest_preserves_binary_boundaries_and_zero_capacity_is_not_eof() {
        use tokio::io::{AsyncRead, AsyncReadExt};
        let digest = Arc::new(Mutex::new(ReadDigest::default()));
        let mut reader = MeasuredAsyncReader {
            reader: Box::pin(&b"\x00\xff\x01\x00"[..]),
            digest: digest.clone(),
        };
        std::future::poll_fn(|context| {
            let mut empty = [];
            let mut buffer = tokio::io::ReadBuf::new(&mut empty);
            std::pin::Pin::new(&mut reader).poll_read(context, &mut buffer)
        })
        .await
        .unwrap();
        assert!(!digest.lock().unwrap().eof);
        let mut byte = [0];
        while reader.read(&mut byte).await.unwrap() != 0 {}
        let result = digest.lock().unwrap();
        assert_eq!(result.bytes, 4);
        assert_eq!(result.checksum, 0xdd85_9b09);
        assert!(result.eof);
    }
    #[test]
    fn digest_measures_arbitrary_binary_read_boundaries_without_retaining_content() {
        let digest = Arc::new(Mutex::new(ReadDigest::default()));
        let mut reader = MeasuredReader {
            reader: Box::new(Cursor::new(vec![0, 255, 1, 0])),
            digest: digest.clone(),
        };
        let mut byte = [0];
        while reader.read(&mut byte).unwrap() != 0 {}
        let result = digest.lock().unwrap();
        assert_eq!(result.bytes, 4);
        assert_eq!(result.checksum, 0xdd85_9b09); // Shared terminal-contract fixture.
        assert!(result.eof);
    }
}
