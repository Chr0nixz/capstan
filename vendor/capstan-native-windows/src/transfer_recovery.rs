//! Profile-encrypted, exclusively held transfer recovery slots.
use crate::{
    editor::{recovery::crypt_bounded, restrict_directory},
    transfer::{version, AnchoredPath},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::fs::OpenOptionsExt,
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_OPEN_REPARSE_POINT,
};
use zeroize::Zeroizing;

pub const SLOTS: [&str; 2] = ["transfer-recovery.0.json", "transfer-recovery.1.json"];
const MAGIC: &[u8] = b"CAPSTAN_TRANSFER_DPAPI_V2\0";
const MAX_CLEAR: usize = 256 * 1024;
const MAX_SEALED: usize = MAX_CLEAR + 32 * 1024;

pub struct RecoveryFrame {
    pub bytes: Zeroizing<Vec<u8>>,
    pub legacy: bool,
}

pub struct RecoveryLedger {
    files: [File; 2],
    entropy: Zeroizing<Vec<u8>>,
    _anchor: AnchoredPath,
}

impl std::fmt::Debug for RecoveryLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecoveryLedger")
            .finish_non_exhaustive()
    }
}

impl RecoveryLedger {
    pub fn open(directory: &Path) -> io::Result<Self> {
        // Pin existing ancestors before creating this one application-owned
        // directory. The second anchor also pins the ledger root itself.
        let parent = AnchoredPath::new(directory)?;
        match fs::create_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let anchor = AnchoredPath::new(&directory.join(SLOTS[0]))?;
        restrict_directory(directory)?;
        let root = OpenOptions::new()
            .access_mode(windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES)
            .share_mode(
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
            )
            .custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT,
            )
            .open(directory)?;
        let identity = version(&root, true)?.identity;
        let open = |name| {
            let path = directory.join(name);
            let options = || {
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .write(true)
                    .share_mode(0)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
                options
            };
            let file = match options().create_new(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    options().open(&path)?
                }
                Err(error) => return Err(error),
            };
            single_file(&file)?;
            Ok(file)
        };
        let files = [open(SLOTS[0])?, open(SLOTS[1])?];
        drop(parent);
        Ok(Self {
            files,
            entropy: Zeroizing::new(
                format!("CapstanTransferRecoveryV2\0{identity:032x}\0").into_bytes(),
            ),
            _anchor: anchor,
        })
    }

    fn entropy(&self, slot: usize) -> io::Result<Zeroizing<Vec<u8>>> {
        if slot >= self.files.len() {
            return Err(invalid());
        }
        let mut entropy = self.entropy.clone();
        entropy.push(slot as u8);
        Ok(entropy)
    }

    pub fn read(&mut self, slot: usize) -> io::Result<Option<RecoveryFrame>> {
        let entropy = self.entropy(slot)?;
        let file = &mut self.files[slot];
        single_file(file)?;
        let length = version(file, false)?.size;
        if length == 0 {
            return Ok(None);
        }
        if length > MAX_SEALED as u64 {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(length as usize));
        file.take(MAX_SEALED as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_SEALED {
            return Err(invalid());
        }
        let (bytes, legacy) = if let Some(sealed) = bytes.strip_prefix(MAGIC) {
            (crypt_bounded(sealed, &entropy, false, MAX_SEALED)?, false)
        } else {
            (bytes, true)
        };
        if bytes.len() > MAX_CLEAR {
            return Err(invalid());
        }
        Ok(Some(RecoveryFrame { bytes, legacy }))
    }

    pub fn write(&mut self, slot: usize, clear: &[u8]) -> io::Result<()> {
        if clear.len() > MAX_CLEAR {
            return Err(invalid());
        }
        let entropy = self.entropy(slot)?;
        let sealed = crypt_bounded(clear, &entropy, true, MAX_SEALED)?;
        if sealed.len() + MAGIC.len() > MAX_SEALED {
            return Err(invalid());
        }
        let file = &mut self.files[slot];
        single_file(file)?;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(MAGIC)?;
        file.write_all(&sealed)?;
        file.sync_all()
    }
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid transfer recovery storage",
    )
}

fn single_file(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    version(file, false)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file owns the live handle and info is a writable Win32 structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.nNumberOfLinks != 1 {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_encrypt_paths_hold_exclusive_files_and_reject_context_substitution() {
        let root = tempfile::tempdir().unwrap();
        let secret = b"private-host /private/remote/file C:\\private\\download.bin";
        let mut ledger = RecoveryLedger::open(root.path()).unwrap();
        ledger.write(0, secret).unwrap();
        ledger.write(1, b"other slot").unwrap();
        assert_eq!(ledger.read(0).unwrap().unwrap().bytes.as_slice(), secret);
        assert!(RecoveryLedger::open(root.path()).is_err());
        assert!(fs::read(root.path().join(SLOTS[0])).is_err());
        assert!(ledger.write(0, &vec![0; MAX_CLEAR + 1]).is_err());
        drop(ledger);
        let sealed = fs::read(root.path().join(SLOTS[0])).unwrap();
        assert!(!sealed.windows(secret.len()).any(|bytes| bytes == secret));
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join(SLOTS[0]), &sealed).unwrap();
        assert!(RecoveryLedger::open(other.path()).unwrap().read(0).is_err());
        fs::write(root.path().join(SLOTS[1]), &sealed).unwrap();
        let mut ledger = RecoveryLedger::open(root.path()).unwrap();
        assert!(ledger.read(1).is_err());
        assert_eq!(ledger.read(0).unwrap().unwrap().bytes.as_slice(), secret);
    }

    #[test]
    fn tampered_oversized_and_linked_slots_do_not_become_recovery_payloads() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = RecoveryLedger::open(root.path()).unwrap();
        ledger.write(0, b"original").unwrap();
        drop(ledger);
        let path = root.path().join(SLOTS[0]);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        let mut ledger = RecoveryLedger::open(root.path()).unwrap();
        assert!(ledger.read(0).is_err());
        drop(ledger);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_SEALED as u64 + 1)
            .unwrap();
        assert!(RecoveryLedger::open(root.path()).unwrap().read(0).is_err());
        let linked = tempfile::tempdir().unwrap();
        let outside = linked.path().join("outside");
        fs::write(&outside, b"preserve outside").unwrap();
        let store = linked.path().join("ledger");
        fs::create_dir(&store).unwrap();
        fs::hard_link(&outside, store.join(SLOTS[0])).unwrap();
        assert!(RecoveryLedger::open(&store).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"preserve outside");
    }
}
