#[cfg(windows)]
pub(super) use capstan_native_windows::transfer_recovery::RecoveryLedger as Storage;

#[cfg(not(windows))]
mod unsupported {
    use std::{io, path::Path};
    use zeroize::Zeroizing;

    #[derive(Debug)]
    pub(crate) struct Storage;
    pub(crate) struct RecoveryFrame {
        pub bytes: Zeroizing<Vec<u8>>,
        pub legacy: bool,
    }

    fn unavailable() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "encrypted transfer recovery is unavailable",
        )
    }

    impl Storage {
        pub fn open(_: &Path) -> io::Result<Self> {
            Err(unavailable())
        }
        pub fn read(&mut self, _: usize) -> io::Result<Option<RecoveryFrame>> {
            Err(unavailable())
        }
        pub fn write(&mut self, _: usize, _: &[u8]) -> io::Result<()> {
            Err(unavailable())
        }
    }
}
#[cfg(not(windows))]
pub(super) use unsupported::Storage;
