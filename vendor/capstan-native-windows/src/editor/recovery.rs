//! Two held recovery slots. Encryption and disk access stay in the native boundary.
use super::{delete_handle, invalid};
use crate::transfer::version;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{LocalFree, GENERIC_READ, GENERIC_WRITE},
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
    Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE,
        FILE_FLAG_OPEN_REPARSE_POINT,
    },
};
use zeroize::{Zeroize, Zeroizing};

pub(super) const SLOTS: [&str; 2] = [".capstan-recovery-a", ".capstan-recovery-b"];
const MAGIC: &[u8] = b"CAPSTAN_EDITOR_DPAPI_V1\0";
const MAX_CLEAR: usize = 64 * 1024;
const MAX_SEALED: u64 = 96 * 1024;

pub struct RecoveryCandidates {
    pub payloads: Vec<RecoveryCandidate>,
    pub corrupt: bool,
}
pub struct RecoveryCandidate {
    pub sequence: u64,
    pub bytes: Zeroizing<Vec<u8>>,
}

pub(super) struct RecoveryFiles {
    files: [Option<File>; 2],
    entropy: Zeroizing<Vec<u8>>,
    highest_sequence: Option<u64>,
}

impl RecoveryFiles {
    pub fn open(root: &Path, name: &str) -> io::Result<Self> {
        let session = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid)?;
        let mut files = [None, None];
        for (index, slot) in SLOTS.iter().enumerate() {
            let open = || {
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .write(true)
                    .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                    .share_mode(0)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
                options
            };
            let path = root.join(slot);
            let file = match open().create_new(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => open().open(&path)?,
                Err(error) => return Err(error),
            };
            single_file(&file)?;
            files[index] = Some(file);
        }
        Ok(Self {
            files,
            entropy: Zeroizing::new(
                format!("CapstanEditorRecoveryV1\0{session}\0{name}").into_bytes(),
            ),
            highest_sequence: None,
        })
    }

    pub fn save(&mut self, sequence: u64, clear: &[u8]) -> io::Result<()> {
        if clear.len() > MAX_CLEAR {
            return Err(invalid());
        }
        if self
            .highest_sequence
            .is_some_and(|latest| sequence <= latest)
        {
            return Err(invalid());
        }
        let mut framed = Zeroizing::new(sequence.to_le_bytes().to_vec());
        framed.extend_from_slice(clear);
        let sealed = crypt(&framed, &self.entropy, true)?;
        if sealed.len() as u64 + MAGIC.len() as u64 > MAX_SEALED {
            return Err(invalid());
        }
        let file = self.files[(sequence % 2) as usize]
            .as_mut()
            .ok_or_else(invalid)?;
        single_file(file)?;
        // Only the inactive slot is overwritten. Core validates the sequence,
        // frame hash and schema; the other synced slot survives a torn write.
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(MAGIC)?;
        file.write_all(&sealed)?;
        file.sync_all()?;
        self.highest_sequence = Some(sequence);
        Ok(())
    }

    pub fn load(&mut self) -> io::Result<RecoveryCandidates> {
        let mut result = RecoveryCandidates {
            payloads: Vec::new(),
            corrupt: false,
        };
        for (index, file) in self.files.iter_mut().enumerate() {
            let Some(file) = file else {
                continue;
            };
            single_file(file)?;
            let length = version(file, false)?.size;
            if length == 0 {
                continue;
            }
            if length > MAX_SEALED {
                result.corrupt = true;
                continue;
            }
            file.seek(SeekFrom::Start(0))?;
            let mut sealed = Vec::with_capacity(length as usize);
            file.take(MAX_SEALED + 1).read_to_end(&mut sealed)?;
            let clear = sealed
                .strip_prefix(MAGIC)
                .ok_or_else(invalid)
                .and_then(|sealed| crypt(sealed, &self.entropy, false));
            match clear {
                Ok(clear) if clear.len() >= 8 && clear.len() <= MAX_CLEAR + 8 => {
                    let sequence =
                        u64::from_le_bytes(clear[..8].try_into().map_err(|_| invalid())?);
                    if (sequence % 2) as usize != index {
                        result.corrupt = true;
                        continue;
                    }
                    self.highest_sequence = Some(
                        self.highest_sequence
                            .map_or(sequence, |latest| latest.max(sequence)),
                    );
                    result.payloads.push(RecoveryCandidate {
                        sequence,
                        bytes: Zeroizing::new(clear[8..].to_vec()),
                    });
                }
                _ => result.corrupt = true,
            }
        }
        Ok(result)
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        for file in self.files.iter().flatten() {
            single_file(file)?;
        }
        for file in &mut self.files {
            if let Some(owned) = file.as_ref() {
                delete_handle(owned)?;
            }
            file.take();
        }
        Ok(())
    }
}

pub(super) fn single_file(file: &File) -> io::Result<()> {
    version(file, false)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file is a live held handle; info is writable and correctly sized.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.nNumberOfLinks != 1 {
        return Err(invalid());
    }
    Ok(())
}

struct Allocation(CRYPT_INTEGER_BLOB);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // SAFETY: a successful DPAPI call owns cbData bytes allocated with
            // LocalAlloc. Clear that buffer before releasing it with LocalFree.
            unsafe {
                std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                LocalFree(self.0.pbData.cast());
            }
        }
    }
}

fn crypt(bytes: &[u8], entropy: &[u8], protect: bool) -> io::Result<Zeroizing<Vec<u8>>> {
    crypt_bounded(bytes, entropy, protect, MAX_SEALED as usize)
}

pub(crate) fn crypt_bounded(
    bytes: &[u8],
    entropy: &[u8],
    protect: bool,
    maximum: usize,
) -> io::Result<Zeroizing<Vec<u8>>> {
    if bytes.len() > maximum || maximum > 512 * 1024 {
        return Err(invalid());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into().map_err(|_| invalid())?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy.len().try_into().map_err(|_| invalid())?,
        pbData: entropy.as_ptr().cast_mut(),
    };
    let mut output = Allocation(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    });
    // SAFETY: input/entropy buffers remain live and are treated as read-only by
    // DPAPI. Null description/prompt/reserved pointers request no UI or audit
    // text. LOCAL_MACHINE is deliberately absent: protection is user-scoped.
    let success = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        }
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    if output.0.pbData.is_null() || output.0.cbData as usize > maximum {
        return Err(invalid());
    }
    // SAFETY: DPAPI returned a valid allocation owned by output until Drop.
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec(),
    ))
}
