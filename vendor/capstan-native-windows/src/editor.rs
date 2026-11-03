//! Owned editor documents. Paths and process arguments never enter the WebView.
use crate::transfer::{valid_name, version, AnchoredPath, Destination, Version};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, GENERIC_READ},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, DELETE, FILE_DISPOSITION_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL, WRITE_DAC,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MARKER: &str = ".capstan-editor";
const OWNER: &str = ".capstan-owner";
const MAGIC: &[u8] = b"CAPSTAN_EDITOR_V1\0";
static COPY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub(crate) mod recovery;
pub use recovery::{RecoveryCandidate, RecoveryCandidates};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub modified: u64,
    pub identity: u128,
}
impl From<Version> for Fingerprint {
    fn from(value: Version) -> Self {
        Self {
            size: value.size,
            modified: value.modified,
            identity: value.identity,
        }
    }
}
pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub fingerprint: Fingerprint,
}

#[derive(Clone)]
pub struct EditorStore {
    root: PathBuf,
}
impl Default for EditorStore {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("Capstan").join("editor"))
    }
}
pub struct Discovery {
    pub stages: Vec<(String, EditorStage)>,
    pub incomplete: bool,
}
impl EditorStore {
    /// Only trusted Rust startup code may choose the storage root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn prepare(&self, session: &str, name: &str, bytes: &[u8]) -> io::Result<EditorStage> {
        if !valid_session(session) || !valid_document_name(name) || bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        ensure_root(&self.root)?;
        let _parent = AnchoredPath::new(&self.root.join(session))?;
        let root = self.root.join(session);
        fs::create_dir(&root)?;
        let anchor = AnchoredPath::new(&root.join(name))?;
        restrict_directory(&root)
            .map_err(|error| io::Error::new(error.kind(), format!("editor ACL: {error}")))?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .access_mode(GENERIC_READ | DELETE)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(root.join(OWNER))?;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(root.join(name))?;
            file.write_all(bytes)?;
            file.sync_all()?;
            let mut marker = OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(root.join(MARKER))?;
            marker.write_all(MAGIC)?;
            marker.write_all(name.as_bytes())?;
            marker.sync_all()
        })();
        let mut stage = EditorStage {
            root,
            name: name.to_owned(),
            anchor: Some(anchor),
            owner: Some(owner),
            recovery: None,
        };
        if let Err(error) = result {
            let _ = stage.try_cleanup(None);
            return Err(error);
        }
        stage.recovery = Some(recovery::RecoveryFiles::open(&stage.root, &stage.name)?);
        Ok(stage)
    }

    /// Claim only marked, inactive sessions. The exclusive owner handle also
    /// excludes other running instances. Unknown trees are never removed.
    pub fn discover(&self, maximum: usize) -> io::Result<Discovery> {
        ensure_root(&self.root)?;
        let _parent = AnchoredPath::new(&self.root.join(MARKER))?;
        let mut result = Discovery {
            stages: Vec::new(),
            incomplete: false,
        };
        for (index, entry) in fs::read_dir(&self.root)?.enumerate() {
            if index >= 256 {
                result.incomplete = true;
                break;
            }
            let entry = entry?;
            let Some(session) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_session(&session) {
                continue;
            }
            match self.claim(&session) {
                Ok(stage) => {
                    if result.stages.len() >= maximum {
                        result.incomplete = true;
                        break;
                    }
                    result.stages.push((session, stage));
                }
                Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {}
                Err(_) => result.incomplete = true,
            }
        }
        Ok(result)
    }

    fn claim(&self, session: &str) -> io::Result<EditorStage> {
        let root = self.root.join(session);
        let anchor = AnchoredPath::new(&root.join(MARKER))?;
        let owner = OpenOptions::new()
            .read(true)
            .access_mode(GENERIC_READ | DELETE)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(root.join(OWNER))?;
        version(&owner, false)?;
        let marker = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(root.join(MARKER))?;
        let metadata = version(&marker, false)?;
        if metadata.size > 1024 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        marker.take(1025).read_to_end(&mut bytes)?;
        let name = bytes
            .strip_prefix(MAGIC)
            .and_then(|name| std::str::from_utf8(name).ok())
            .filter(|name| valid_document_name(name))
            .ok_or_else(invalid)?;
        restrict_directory(&root)?;
        let recovery = recovery::RecoveryFiles::open(&root, name)?;
        Ok(EditorStage {
            root,
            name: name.to_owned(),
            anchor: Some(anchor),
            owner: Some(owner),
            recovery: Some(recovery),
        })
    }
}

pub struct EditorStage {
    root: PathBuf,
    name: String,
    anchor: Option<AnchoredPath>,
    owner: Option<File>,
    recovery: Option<recovery::RecoveryFiles>,
}
impl EditorStage {
    pub fn save_recovery(&mut self, sequence: u64, clear: &[u8]) -> io::Result<()> {
        self.recovery
            .as_mut()
            .ok_or_else(invalid)?
            .save(sequence, clear)
    }
    pub fn load_recovery(&mut self) -> io::Result<RecoveryCandidates> {
        self.recovery.as_mut().ok_or_else(invalid)?.load()
    }
    pub fn prepare(session: &str, name: &str, bytes: &[u8]) -> io::Result<Self> {
        EditorStore::default().prepare(session, name, bytes)
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> PathBuf {
        self.root.join(&self.name)
    }
    /// Deny concurrent writes/deletes during the bounded read. Check the
    /// actual handle, including reparse attributes, after opening it.
    pub fn snapshot(&self) -> io::Result<Snapshot> {
        if self.owner.is_none() {
            return Err(invalid());
        }
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path())?;
        let before = version(&file, false)?;
        recovery::single_file(&file)?;
        if before.size > MAX_BYTES {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(before.size as usize);
        (&file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES || before != version(&file, false)? {
            return Err(invalid());
        }
        Ok(Snapshot {
            bytes,
            fingerprint: before.into(),
        })
    }
    pub fn read(&self) -> io::Result<Vec<u8>> {
        Ok(self.snapshot()?.bytes)
    }
    pub fn fingerprint(&self) -> io::Result<Fingerprint> {
        Ok(self.snapshot()?.fingerprint)
    }

    /// One configured executable and one separate argv entry, without a shell.
    pub fn launch(&self, executable: &Path) -> io::Result<Child> {
        let _anchor = AnchoredPath::new(executable)?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(executable)?;
        version(&file, false)?;
        if !executable
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            return Err(invalid());
        }
        Command::new(executable)
            .arg(self.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    /// Do not traverse unknown children or kill editor child processes. Keep
    /// the ownership marker if a sharing/type preflight fails.
    pub fn try_cleanup(&mut self, expected: Option<Fingerprint>) -> io::Result<()> {
        for entry in fs::read_dir(&self.root)? {
            let name = entry?.file_name();
            if name != self.name.as_str()
                && name != MARKER
                && name != OWNER
                && !recovery::SLOTS.iter().any(|slot| name == *slot)
            {
                return Err(invalid());
            }
        }
        let document = match open_for_delete(&self.path()) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound && expected.is_none() => None,
            Err(error) => return Err(error),
        };
        if let Some(file) = &document {
            recovery::single_file(file)?;
            let actual: Fingerprint = version(file, false)?.into();
            if expected.is_some_and(|expected| expected != actual) {
                return Err(invalid());
            }
        }
        let marker = match open_for_delete(&self.root.join(MARKER)) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if let Some(document) = document {
            delete_handle(&document)?;
        }
        if let Some(marker) = marker {
            delete_handle(&marker)?;
        }
        if let Some(recovery) = &mut self.recovery {
            recovery.cleanup()?;
        }
        if let Some(owner) = &self.owner {
            delete_handle(owner)?;
        }
        self.owner.take();
        self.anchor.take();
        fs::remove_dir(&self.root)
    }
    pub fn cleanup(mut self) -> io::Result<()> {
        self.try_cleanup(None)
    }
}

/// Use the same anchored, exclusive, synced publication as normal downloads.
pub fn copy_bytes_to_folder(root: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    if !valid_name(name) || bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    let mut destination = Destination::select_under_folder(root, Path::new(name))?;
    if destination.snapshot()?.is_some() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    let sequence = COPY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    destination.begin(&format!(
        ".capstan-edit-copy-{}-{sequence}.partial",
        std::process::id()
    ))?;
    let result = (|| {
        for (index, bytes) in bytes.chunks(32768).enumerate() {
            destination.write((index * 32768) as u64, bytes)?;
        }
        destination.commit(None)
    })();
    if result.is_err() {
        let _ = destination.discard();
    }
    result
}
fn open_for_delete(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    version(&file, false)?;
    Ok(file)
}

/// Protect the random session directory before writing any document bytes.
/// Child files inherit access for the current Windows user and SYSTEM only.
pub(crate) fn restrict_directory(path: &Path) -> io::Result<()> {
    let directory = OpenOptions::new()
        .access_mode(WRITE_DAC | READ_CONTROL | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| io::Error::new(error.kind(), format!("open: {error}")))?;
    version(&directory, true)
        .map_err(|error| io::Error::new(error.kind(), format!("version: {error}")))?;
    let user = current_user_sid()
        .map_err(|error| io::Error::new(error.kind(), format!("user SID: {error}")))?;
    let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: sddl is NUL-terminated; the API allocates the returned descriptor.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    // SAFETY: descriptor is the valid allocated descriptor above; output pointers
    // stay live. SetSecurityInfo copies the ACL before LocalFree releases it.
    let result = unsafe {
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) == 0
            || present == 0
            || dacl.is_null()
        {
            Err(invalid())
        } else {
            let code = SetSecurityInfo(
                directory.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            );
            if code == 0 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("set DACL: {}", io::Error::from_raw_os_error(code as i32)),
                ))
            }
        }
    };
    // SAFETY: descriptor was allocated by the conversion API and is freed once.
    unsafe {
        LocalFree(descriptor);
    }
    result
}

fn current_user_sid() -> io::Result<String> {
    let mut token = std::ptr::null_mut();
    // SAFETY: the process pseudo-handle is valid and the returned token handle
    // is released after every result from the closure below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut required = 0;
        // SAFETY: the first query requests the bounded size for the process token's
        // user record; token remains live through both queries.
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
        }
        if required == 0 || required > 64 * 1024 {
            return Err(invalid());
        }
        let mut buffer = vec![0_usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
        // SAFETY: aligned storage covers required bytes and remains live while its
        // TOKEN_USER/SID is read. The string SID is separately allocated by Win32.
        unsafe {
            if GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut sid = std::ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut length = 0;
            while length < 256 && *sid.add(length) != 0 {
                length += 1;
            }
            let result = if length < 256 {
                String::from_utf16(std::slice::from_raw_parts(sid, length)).map_err(|_| invalid())
            } else {
                Err(invalid())
            };
            LocalFree(sid.cast());
            result
        }
    })();
    // SAFETY: OpenProcessToken returned this owned handle, closed exactly once.
    unsafe {
        CloseHandle(token);
    }
    result
}
fn delete_handle(file: &File) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: file owns a live handle and the correctly sized value stays live.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of_val(&disposition) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn ensure_root(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(invalid)?;
    if !parent.exists() {
        ensure_root(parent)?;
    }
    let _anchor = AnchoredPath::new(&path.join(MARKER)).or_else(|error| {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error);
        }
        let _parent = AnchoredPath::new(path)?;
        fs::create_dir(path)?;
        AnchoredPath::new(&path.join(MARKER))
    })?;
    Ok(())
}
fn valid_document_name(value: &str) -> bool {
    valid_name(value)
        && !value.eq_ignore_ascii_case(MARKER)
        && !value.eq_ignore_ascii_case(OWNER)
        && !recovery::SLOTS
            .iter()
            .any(|slot| value.eq_ignore_ascii_case(slot))
}
fn valid_session(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid editor capability")
}

#[cfg(test)]
mod tests {
    use super::*;
    const SESSION: &str = "00000000-0000-4000-8000-000000000001";
    fn store(root: &Path) -> EditorStore {
        EditorStore::new(root.join("editor"))
    }

    #[test]
    fn saves_and_rename_saves_are_observed_without_locking_the_document() {
        let root = tempfile::tempdir().unwrap();
        let mut stage = store(root.path())
            .prepare(SESSION, "notes 中文.txt", b"hello\n")
            .unwrap();
        let before = stage.snapshot().unwrap();
        let replacement = stage.path().with_file_name("replacement.txt");
        fs::write(&replacement, b"world\n").unwrap();
        fs::remove_file(stage.path()).unwrap();
        fs::rename(replacement, stage.path()).unwrap();
        let after = stage.snapshot().unwrap();
        assert_eq!(after.bytes, b"world\n");
        assert_ne!(before.fingerprint.identity, after.fingerprint.identity);
        assert!(stage.try_cleanup(Some(before.fingerprint)).is_err());
        assert_eq!(stage.read().unwrap(), b"world\n");
        stage.cleanup().unwrap();
    }

    #[test]
    fn discovery_excludes_live_instances_and_claims_only_marked_stages() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let stage = store.prepare(SESSION, "config.txt", b"unsent").unwrap();
        assert!(store.discover(8).unwrap().stages.is_empty());
        assert!(!store.discover(0).unwrap().incomplete);
        drop(stage);
        let mut discovered = store.discover(8).unwrap();
        assert_eq!(discovered.stages.len(), 1);
        assert_eq!(discovered.stages[0].1.read().unwrap(), b"unsent");
        assert!(store.discover(8).unwrap().stages.is_empty());
        discovered.stages.pop().unwrap().1.cleanup().unwrap();
        fs::create_dir(store.root.join(SESSION)).unwrap();
        assert!(store.discover(8).unwrap().stages.is_empty());
        assert!(store.root.join(SESSION).exists());
    }

    #[test]
    fn unknown_children_are_preserved_and_ancestors_cannot_be_replaced() {
        let root = tempfile::tempdir().unwrap();
        let mut stage = store(root.path())
            .prepare(SESSION, "notes.txt", b"local")
            .unwrap();
        fs::write(stage.path().with_file_name("editor-backup"), b"backup").unwrap();
        assert!(stage.try_cleanup(None).is_err());
        assert_eq!(stage.read().unwrap(), b"local");
        assert!(fs::rename(&stage.root, root.path().join("moved")).is_err());
        fs::remove_file(stage.path().with_file_name("editor-backup")).unwrap();
        stage.cleanup().unwrap();
    }

    #[test]
    fn local_copy_is_exclusive_and_rejects_reserved_names_and_escape() {
        let root = tempfile::tempdir().unwrap();
        copy_bytes_to_folder(root.path(), "copy.txt", b"saved").unwrap();
        assert!(copy_bytes_to_folder(root.path(), "copy.txt", b"replace").is_err());
        assert!(copy_bytes_to_folder(root.path(), "../escape", b"x").is_err());
        assert!(copy_bytes_to_folder(root.path(), "NUL.txt", b"x").is_err());
        assert_eq!(fs::read(root.path().join("copy.txt")).unwrap(), b"saved");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        assert!(store(root.path())
            .prepare("../escape", "ok.txt", b"x")
            .is_err());
        for name in [
            ".capstan-editor",
            ".CAPSTAN-EDITOR",
            ".Capstan-Owner",
            ".CAPSTAN-RECOVERY-A",
            ".capstan-recovery-b",
        ] {
            assert!(store(root.path()).prepare(SESSION, name, b"x").is_err());
            assert!(!root.path().join("editor").exists());
        }
    }

    #[test]
    fn forged_markers_never_authorize_reading_or_removing_another_path() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let stage = store.prepare(SESSION, "notes.txt", b"retained").unwrap();
        let marker = stage.root.join(MARKER);
        drop(stage);
        for name in [
            "../outside",
            "C:\\outside.txt",
            "notes.txt:stream",
            ".capstan-owner",
            ".CAPSTAN-OWNER",
            ".Capstan-Editor",
        ] {
            fs::write(&marker, [MAGIC, name.as_bytes()].concat()).unwrap();
            let discovered = store.discover(8).unwrap();
            assert!(discovered.stages.is_empty());
            assert!(discovered.incomplete);
            assert_eq!(
                fs::read(store.root.join(SESSION).join("notes.txt")).unwrap(),
                b"retained"
            );
        }
    }

    #[test]
    fn encrypted_checkpoints_are_bounded_pinned_and_survive_a_torn_slot() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let mut stage = store.prepare(SESSION, "notes.txt", b"document").unwrap();
        let secret = b"private-host:/private/remote/path";
        stage.save_recovery(1, secret).unwrap();
        stage.save_recovery(2, b"newer checkpoint").unwrap();
        assert!(stage.save_recovery(1, b"old caller").is_err());
        assert!(stage.save_recovery(3, &vec![0; 65537]).is_err());
        assert!(fs::read(stage.root.join(recovery::SLOTS[0])).is_err());
        let session_root = stage.root.clone();
        drop(stage);
        let sealed = fs::read(session_root.join(recovery::SLOTS[1])).unwrap();
        assert!(!sealed.windows(secret.len()).any(|window| window == secret));
        fs::write(session_root.join(recovery::SLOTS[0]), b"torn checkpoint").unwrap();
        let mut recovered = store.discover(8).unwrap().stages.pop().unwrap().1;
        let loaded = recovered.load_recovery().unwrap();
        assert!(loaded.corrupt);
        assert_eq!(loaded.payloads.len(), 1);
        assert_eq!(&*loaded.payloads[0].bytes, secret);
        recovered.save_recovery(3, b"repaired").unwrap();
        recovered.save_recovery(4, b"complete").unwrap();
        assert!(!recovered.load_recovery().unwrap().corrupt);
        recovered.cleanup().unwrap();
        assert!(!session_root.exists());
    }

    #[test]
    fn encrypted_checkpoints_cannot_be_moved_between_editor_sessions() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let other = "00000000-0000-4000-8000-000000000002";
        let mut first = store.prepare(SESSION, "notes.txt", b"first").unwrap();
        let second = store.prepare(other, "notes.txt", b"second").unwrap();
        first.save_recovery(1, b"private metadata").unwrap();
        drop(first);
        drop(second);
        fs::copy(
            store.root.join(SESSION).join(recovery::SLOTS[1]),
            store.root.join(other).join(recovery::SLOTS[1]),
        )
        .unwrap();
        for (id, mut stage) in store.discover(8).unwrap().stages {
            let loaded = stage.load_recovery().unwrap();
            if id == other {
                assert!(loaded.corrupt);
                assert!(loaded.payloads.is_empty());
            } else {
                assert!(!loaded.corrupt);
                assert_eq!(loaded.payloads.len(), 1);
            }
            stage.cleanup().unwrap();
        }
    }

    #[test]
    fn recovery_slots_reject_hard_links_without_touching_the_external_file() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let stage = store.prepare(SESSION, "notes.txt", b"retained").unwrap();
        let slot = stage.root.join(recovery::SLOTS[0]);
        drop(stage);
        let external = root.path().join("external.txt");
        fs::write(&external, b"external data").unwrap();
        fs::remove_file(&slot).unwrap();
        fs::hard_link(&external, &slot).unwrap();
        let discovery = store.discover(8).unwrap();
        assert!(discovery.incomplete);
        assert!(discovery.stages.is_empty());
        assert_eq!(fs::read(&external).unwrap(), b"external data");
        assert_eq!(
            fs::read(store.root.join(SESSION).join("notes.txt")).unwrap(),
            b"retained"
        );
    }

    #[test]
    fn a_document_replaced_with_a_hard_link_is_neither_read_nor_deleted() {
        let root = tempfile::tempdir().unwrap();
        let mut stage = store(root.path())
            .prepare(SESSION, "notes.txt", b"original")
            .unwrap();
        let external = root.path().join("private.txt");
        fs::write(&external, b"outside the staging tree").unwrap();
        fs::remove_file(stage.path()).unwrap();
        fs::hard_link(&external, stage.path()).unwrap();
        assert!(stage.snapshot().is_err());
        assert!(stage.try_cleanup(None).is_err());
        assert!(stage.path().exists());
        assert_eq!(fs::read(&external).unwrap(), b"outside the staging tree");
        fs::remove_file(stage.path()).unwrap();
        stage.cleanup().unwrap();
    }

    #[test]
    fn editor_output_is_not_inherited_by_the_application() {
        const VARIABLE: &str = "CAPSTAN_TEST_EDITOR_OUTPUT_ROOT";
        if let Some(root) = std::env::var_os(VARIABLE) {
            let stage = store(Path::new(&root))
                .prepare(SESSION, "notes.txt", b"document")
                .unwrap();
            // A nonmatching document-path filter makes this test executable
            // print its zero-test report. That output must not reach our app.
            let mut child = stage.launch(&std::env::current_exe().unwrap()).unwrap();
            assert!(child.wait().unwrap().success());
            stage.cleanup().unwrap();
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "editor::tests::editor_output_is_not_inherited_by_the_application",
            ])
            .env(VARIABLE, root.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("running 0 tests"));
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn staging_directory_has_a_protected_user_and_system_acl() {
        use windows_sys::Win32::Security::{
            Authorization::GetSecurityInfo, GetSecurityDescriptorControl, SE_DACL_PROTECTED,
        };
        let root = tempfile::tempdir().unwrap();
        let stage = store(root.path())
            .prepare(SESSION, "private.txt", b"private")
            .unwrap();
        let directory = OpenOptions::new()
            .access_mode(READ_CONTROL)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&stage.root)
            .unwrap();
        let mut descriptor = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: all outputs and the directory handle stay live; the API's
        // descriptor allocation is freed after inspecting its contained ACL.
        unsafe {
            assert_eq!(
                GetSecurityInfo(
                    directory.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut dacl,
                    std::ptr::null_mut(),
                    &mut descriptor
                ),
                0
            );
            assert_ne!(
                GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
                0
            );
            assert_ne!(control & SE_DACL_PROTECTED, 0);
            assert!(!dacl.is_null());
            assert_eq!((*dacl).AceCount, 2);
            LocalFree(descriptor);
        }
        drop(directory);
        stage.cleanup().unwrap();
    }
}
