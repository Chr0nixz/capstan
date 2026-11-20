//! Unix counterpart of `capstan_native_windows::transfer`.
//!
//! Picker ancestors are resolved once (macOS `/var` and `/tmp` are links).
//! Mutations and reads use directory-relative syscalls after a link-free
//! reopen and identity check. Queued sources hold no descriptor until read.
use rustix::fs::{self as at, AtFlags, Dir, FileType, Mode, OFlags, Stat};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    os::unix::{ffi::OsStrExt, fs::FileExt},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

const MAX_IO: usize = 32_768;
const MAX_COMPONENTS: usize = 64;
const MAX_PATH_BYTES: usize = 4096;
const MAX_TREE_ENTRIES: usize = 4096;
const FILE_TYPE_MASK: u32 = 0o170_000;
const DIRECTORY_TYPE: u32 = 0o040_000;
const REGULAR_TYPE: u32 = 0o100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub size: u64,
    pub modified: u64,
    pub identity: u128,
    pub attributes: u32,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid local file capability")
}
fn conflict() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "local target changed")
}

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// A single portable local path component. Unix accepts more than Windows;
/// only separators, control characters and the dot entries are refused.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c.is_control() || c == '/')
}

fn identity(stat: &Stat) -> u128 {
    (u128::from(nonnegative(stat.st_dev)) << 64) | u128::from(stat.st_ino)
}

fn nonnegative(value: impl TryInto<u64>) -> u64 {
    value.try_into().unwrap_or_default()
}

fn version_of(stat: &Stat, kind: FileType) -> io::Result<Version> {
    if FileType::from_raw_mode(stat.st_mode) != kind {
        return Err(invalid());
    }
    let seconds = nonnegative(stat.st_mtime);
    let nanos = nonnegative(stat.st_mtime_nsec);
    Ok(Version {
        size: nonnegative(stat.st_size),
        modified: seconds.saturating_mul(1_000_000_000).saturating_add(nanos),
        identity: identity(stat),
        attributes: u32::try_from(nonnegative(stat.st_mode)).unwrap_or_default(),
    })
}

fn map_open_error(error: rustix::io::Errno) -> io::Error {
    if matches!(error, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
        invalid()
    } else {
        error.into()
    }
}

fn open_regular(parent: &File, name: &OsStr, access: OFlags) -> io::Result<File> {
    let flags = access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    at::openat(parent, name, flags, Mode::RUSR | Mode::WUSR)
        .map(File::from)
        .map_err(map_open_error)
}

#[cfg(target_os = "macos")]
fn open_directory(path: &Path) -> io::Result<File> {
    // Darwin checks every path component in a single lookup. It refuses
    // O_NOFOLLOW combined with O_NOFOLLOW_ANY with EINVAL.
    let no_links = OFlags::from_bits_retain(libc::O_NOFOLLOW_ANY.cast_unsigned());
    let flags = DIRECTORY_FLAGS.difference(OFlags::NOFOLLOW) | no_links;
    at::open(path, flags, Mode::empty())
        .map(File::from)
        .map_err(map_open_error)
}

#[cfg(not(target_os = "macos"))]
fn open_directory(path: &Path) -> io::Result<File> {
    let mut directory = File::from(at::open("/", DIRECTORY_FLAGS, Mode::empty())?);
    for component in path.components().skip(1) {
        directory = open_subdirectory(&directory, component.as_os_str())?;
    }
    Ok(directory)
}

fn open_subdirectory(parent: &File, name: &OsStr) -> io::Result<File> {
    at::openat(parent, name, DIRECTORY_FLAGS, Mode::empty())
        .map(File::from)
        .map_err(map_open_error)
}

/// Absolute path with only normal, valid components; returns the parent and
/// final name.
fn split_absolute(path: &Path) -> io::Result<(&Path, &OsStr)> {
    if !path.is_absolute() || path.as_os_str().len() > MAX_PATH_BYTES {
        return Err(invalid());
    }
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return Err(invalid());
    }
    let mut count = 0;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(invalid());
        };
        if !name.to_str().is_some_and(valid_name) {
            return Err(invalid());
        }
        count += 1;
    }
    if count == 0 || count > MAX_COMPONENTS {
        return Err(invalid());
    }
    let name = path.file_name().ok_or_else(invalid)?;
    let parent = path.parent().ok_or_else(invalid)?;
    Ok((parent, name))
}

fn valid_relative_components(path: &Path) -> io::Result<Vec<OsString>> {
    if path.is_absolute() {
        return Err(invalid());
    }
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(invalid());
        };
        let Some(name) = name.to_str() else {
            return Err(invalid());
        };
        if !valid_name(name) {
            return Err(invalid());
        }
        components.push(OsString::from(name));
    }
    if components.is_empty() || components.len() > MAX_COMPONENTS {
        return Err(invalid());
    }
    Ok(components)
}

/// A real directory chosen by the user. Its own final component must not be a
/// symlink; ancestor links are resolved once.
fn resolve_directory(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(invalid());
    }
    fs::canonicalize(path)
}

/// A file path whose parent directory identity is fixed at creation.
struct AnchoredPath {
    path: PathBuf,
    parent_identity: u128,
}

impl AnchoredPath {
    /// Picker and drop paths: resolve ancestor links once, then pin.
    fn new(path: &Path) -> io::Result<Self> {
        let (parent, name) = split_absolute(path)?;
        let parent = fs::canonicalize(parent)?;
        let directory = open_directory(&parent)?;
        Ok(Self {
            path: parent.join(name),
            parent_identity: identity(&at::fstat(directory)?),
        })
    }

    fn checked_parent(&self) -> io::Result<File> {
        let parent = open_directory(self.path.parent().ok_or_else(invalid)?)?;
        if identity(&at::fstat(&parent)?) != self.parent_identity {
            return Err(conflict());
        }
        Ok(parent)
    }

    fn snapshot(&self) -> io::Result<Option<Version>> {
        snapshot_at(&self.checked_parent()?, self.filename()?)
    }

    fn filename(&self) -> io::Result<&OsStr> {
        self.path.file_name().ok_or_else(invalid)
    }

    fn name(&self) -> io::Result<String> {
        self.filename()?
            .to_str()
            .map(str::to_owned)
            .ok_or_else(invalid)
    }
}

fn snapshot_at(parent: &File, name: &OsStr) -> io::Result<Option<Version>> {
    let stat = match at::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let kind = if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
        FileType::Directory
    } else {
        FileType::RegularFile
    };
    version_of(&stat, kind).map(Some)
}

fn read_exact_at(file: &File, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    if length > MAX_IO {
        return Err(invalid());
    }
    let mut data = vec![0; length];
    let mut filled = 0;
    while filled < length {
        let position = offset.checked_add(filled as u64).ok_or_else(invalid)?;
        match file.read_at(&mut data[filled..], position) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    data.truncate(filled);
    Ok(data)
}

/// A selected regular file. The descriptor is opened lazily and released
/// after [`Source::verify`], so queued sources do not hold descriptors.
pub struct Source {
    anchor: AnchoredPath,
    file: Mutex<Option<File>>,
    baseline: Version,
}

impl Source {
    pub fn open(path: &Path) -> io::Result<Self> {
        let anchor = AnchoredPath::new(path)?;
        let baseline = anchor.snapshot()?.ok_or(io::ErrorKind::NotFound)?;
        if baseline.attributes & FILE_TYPE_MASK != REGULAR_TYPE {
            return Err(invalid());
        }
        let source = Self::from_snapshot(anchor, baseline);
        source.reopen()?;
        Ok(source)
    }

    fn from_snapshot(anchor: AnchoredPath, baseline: Version) -> Self {
        Self {
            anchor,
            file: Mutex::new(None),
            baseline,
        }
    }

    pub fn name(&self) -> io::Result<String> {
        self.anchor.name()
    }

    pub fn size(&self) -> u64 {
        self.baseline.size
    }

    fn reopen(&self) -> io::Result<File> {
        let parent = self.anchor.checked_parent()?;
        let file = open_regular(&parent, self.anchor.filename()?, OFlags::RDONLY)?;
        if version_of(&at::fstat(&file)?, FileType::RegularFile)? != self.baseline {
            return Err(conflict());
        }
        Ok(file)
    }

    pub fn read(&self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        if length > MAX_IO {
            return Err(invalid());
        }
        let mut slot = self.file.lock().map_err(|_| invalid())?;
        if slot.is_none() {
            *slot = Some(self.reopen()?);
        }
        read_exact_at(slot.as_ref().ok_or_else(invalid)?, offset, length)
    }

    /// Confirm the bytes read so far still belong to the selected, unchanged
    /// file, then release the descriptor.
    pub fn verify(&self) -> io::Result<()> {
        let mut slot = self.file.lock().map_err(|_| invalid())?;
        let current = match slot.take() {
            Some(file) => version_of(&at::fstat(&file)?, FileType::RegularFile)?,
            None => version_of(&at::fstat(self.reopen()?)?, FileType::RegularFile)?,
        };
        if current == self.baseline && self.anchor.snapshot()? == Some(self.baseline) {
            Ok(())
        } else {
            Err(conflict())
        }
    }
}

/// A picker-selected target plus at most one owned temporary sibling.
pub struct Destination {
    anchor: AnchoredPath,
    temporary: Option<File>,
    temporary_path: Option<PathBuf>,
    preserve_on_drop: bool,
}

/// A picker-selected folder. Nothing is created under it until a transfer
/// asks for a destination.
pub struct SelectedDirectory {
    root: PathBuf,
    identity: u128,
}

impl SelectedDirectory {
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = resolve_directory(root)?;
        let identity = identity(&at::fstat(open_directory(&root)?)?);
        Ok(Self { root, identity })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot(&self, relative: &Path) -> io::Result<Option<Version>> {
        let components = valid_relative_components(relative)?;
        let (name, parents) = components.split_last().ok_or_else(invalid)?;
        let root = self.checked_root()?;
        match walk_directories(root, parents, false) {
            Ok(directory) => snapshot_at(&directory, name),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn checked_root(&self) -> io::Result<File> {
        let directory = open_directory(&self.root)?;
        if identity(&at::fstat(&directory)?) != self.identity {
            return Err(conflict());
        }
        Ok(directory)
    }

    pub fn prepare(&self, directories: &[PathBuf]) -> io::Result<()> {
        if directories.len() > MAX_TREE_ENTRIES {
            return Err(invalid());
        }
        let root = self.checked_root()?;
        for directory in directories {
            let components = valid_relative_components(directory)?;
            walk_directories(root.try_clone()?, &components, true)?;
        }
        Ok(())
    }

    pub fn select_destination(&self, relative: &Path) -> io::Result<Destination> {
        let components = valid_relative_components(relative)?;
        let (name, parents) = components.split_last().ok_or_else(invalid)?;
        let directory = walk_directories(self.checked_root()?, parents, true)?;
        let anchor = AnchoredPath {
            path: self.root.join(relative),
            parent_identity: identity(&at::fstat(&directory)?),
        };
        snapshot_at(&directory, name)?;
        Ok(Destination::with_anchor(anchor))
    }
}

fn owned_path(parent: &File, name: &OsStr, file: &File) -> io::Result<()> {
    let named = at::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if FileType::from_raw_mode(named.st_mode) != FileType::RegularFile
        || identity(&named) != identity(&at::fstat(file)?)
    {
        return Err(conflict());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_new(parent: &File, temporary: &OsStr, target: &OsStr) -> io::Result<()> {
    match at::renameat_with(
        parent,
        temporary,
        parent,
        target,
        at::RenameFlags::NOREPLACE,
    ) {
        Ok(()) => Ok(()),
        Err(error) if lacks_exclusive_publish(error) => {
            publish_after_check(parent, temporary, target)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(target_os = "macos"))]
fn publish_new(parent: &File, temporary: &OsStr, target: &OsStr) -> io::Result<()> {
    match at::linkat(parent, temporary, parent, target, AtFlags::empty()) {
        Ok(()) => {
            let _ = at::unlinkat(parent, temporary, AtFlags::empty());
            Ok(())
        }
        Err(error) if lacks_exclusive_publish(error) => {
            publish_after_check(parent, temporary, target)
        }
        Err(error) => Err(error.into()),
    }
}

/// exFAT, SMB and similar volumes offer neither `RENAME_EXCL` nor hard links.
fn lacks_exclusive_publish(error: rustix::io::Errno) -> bool {
    use rustix::io::Errno;
    // A list, not a match pattern: Linux defines NOTSUP and OPNOTSUPP as the
    // same value, which would make one pattern unreachable there.
    [
        Errno::NOTSUP,
        Errno::OPNOTSUPP,
        Errno::NOSYS,
        Errno::PERM,
        Errno::MLINK,
    ]
    .contains(&error)
}

/// Best available no-replace publish on those volumes: the target is rechecked
/// right before the rename, leaving only a narrow race with another writer.
fn publish_after_check(parent: &File, temporary: &OsStr, target: &OsStr) -> io::Result<()> {
    if snapshot_at(parent, target)?.is_some() {
        return Err(conflict());
    }
    at::renameat(parent, temporary, parent, target)?;
    Ok(())
}

/// `File::sync_all` is `F_FULLFSYNC` on Darwin, which network and some removable
/// volumes refuse; those still accept a plain `fsync`.
fn sync_file(file: &File) -> io::Result<()> {
    match file.sync_all() {
        // As above, ENOTSUP and EOPNOTSUPP are one value on Linux.
        Err(error)
            if error.raw_os_error().is_some_and(|code| {
                [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EINVAL].contains(&code)
            }) =>
        {
            at::fsync(file).map_err(Into::into)
        }
        result => result,
    }
}

/// macOS starts GUI apps with a soft limit of 256 descriptors, which a few busy
/// SSH sessions and transfers can exhaust. Never lowers an existing limit.
pub fn raise_open_file_limit() -> io::Result<()> {
    use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};
    // Darwin's OPEN_MAX: setrlimit refuses more even with an unlimited hard limit.
    const TARGET: u64 = 10_240;
    let limit = getrlimit(Resource::Nofile);
    let target = limit.maximum.map_or(TARGET, |maximum| maximum.min(TARGET));
    if limit.current.is_none_or(|current| current >= target) {
        return Ok(());
    }
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(target),
            maximum: limit.maximum,
        },
    )
    .map_err(Into::into)
}

impl Destination {
    pub fn select(path: &Path) -> io::Result<Self> {
        let anchor = AnchoredPath::new(path)?;
        anchor.snapshot()?;
        Ok(Self::with_anchor(anchor))
    }

    fn with_anchor(anchor: AnchoredPath) -> Self {
        Self {
            anchor,
            temporary: None,
            temporary_path: None,
            preserve_on_drop: false,
        }
    }

    /// Create a destination below a picker-selected directory. Every parent
    /// component is checked for links before it is created or reused; the
    /// final file may already exist so the conflict workflow can decide.
    pub fn select_under_folder(root: &Path, relative: &Path) -> io::Result<Self> {
        SelectedDirectory::open(root)?.select_destination(relative)
    }

    pub fn name(&self) -> io::Result<String> {
        self.anchor.name()
    }

    /// The target chosen by the native picker, for recovery metadata only.
    pub fn target_path(&self) -> PathBuf {
        self.anchor.path.clone()
    }

    pub fn snapshot(&self) -> io::Result<Option<Version>> {
        self.anchor.snapshot()
    }

    pub fn retarget(&mut self, name: &str) -> io::Result<()> {
        if !valid_name(name) {
            return Err(invalid());
        }
        self.anchor.path.set_file_name(name);
        self.anchor.snapshot().map(|_| ())
    }

    pub fn begin(&mut self, name: &str) -> io::Result<()> {
        if !valid_name(name) || self.temporary.is_some() {
            return Err(invalid());
        }
        let directory = self.anchor.checked_parent()?;
        let file = open_regular(
            &directory,
            OsStr::new(name),
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
        )?;
        self.temporary_path = Some(self.anchor.path.with_file_name(name));
        self.temporary = Some(file);
        Ok(())
    }

    pub fn partial_size(&self) -> io::Result<Option<u64>> {
        self.temporary
            .as_ref()
            .map(|file| {
                at::fstat(file)
                    .map(|stat| nonnegative(stat.st_size))
                    .map_err(Into::into)
            })
            .transpose()
    }

    /// The exact temporary path owned by this destination. Used only by the
    /// Rust recovery ledger, never handed to the WebView.
    pub fn partial_path(&self) -> Option<PathBuf> {
        self.temporary_path.clone()
    }

    pub fn partial_identity(&self) -> io::Result<Option<u128>> {
        self.temporary
            .as_ref()
            .map(|file| {
                at::fstat(file)
                    .map(|stat| identity(&stat))
                    .map_err(Into::into)
            })
            .transpose()
    }

    pub fn preserve_partial(&mut self) {
        self.preserve_on_drop = true;
    }

    /// Reattach only a recorded sibling of a newly picker-selected target.
    pub fn recover_partial(
        &mut self,
        path: &Path,
        identity: u128,
        minimum: u64,
        maximum: u64,
    ) -> io::Result<()> {
        if self.temporary.is_some()
            || path.parent() != self.anchor.path.parent()
            || path == self.anchor.path
            || minimum > maximum
        {
            return Err(invalid());
        }
        let directory = self.anchor.checked_parent()?;
        let name = path
            .file_name()
            .filter(|name| name.to_str().is_some_and(valid_name))
            .ok_or_else(invalid)?;
        let file = open_regular(&directory, name, OFlags::RDWR)?;
        let stat = at::fstat(&file)?;
        let actual = version_of(&stat, FileType::RegularFile)?;
        if actual.identity != identity
            || actual.size < minimum
            || actual.size > maximum
            || stat.st_nlink != 1
        {
            return Err(conflict());
        }
        self.temporary = Some(file);
        self.temporary_path = Some(path.to_owned());
        // An abandoned picker or grant must not delete a pre-existing partial.
        self.preserve_on_drop = true;
        Ok(())
    }

    pub fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        if data.len() > MAX_IO {
            return Err(invalid());
        }
        let file = self.temporary.as_ref().ok_or_else(invalid)?;
        file.write_all_at(data, offset)
    }

    pub fn read(&self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        read_exact_at(self.temporary.as_ref().ok_or_else(invalid)?, offset, length)
    }

    pub fn sync(&self) -> io::Result<()> {
        sync_file(self.temporary.as_ref().ok_or_else(invalid)?)
    }

    pub fn commit(&mut self, expected: Option<Version>) -> io::Result<()> {
        if expected.is_some_and(|version| version.attributes & FILE_TYPE_MASK == DIRECTORY_TYPE) {
            return Err(conflict());
        }
        self.sync()?;
        let directory = self.anchor.checked_parent()?;
        let target = self.anchor.filename()?;
        if snapshot_at(&directory, target)? != expected {
            return Err(conflict());
        }
        let file = self.temporary.as_ref().ok_or_else(invalid)?;
        let temporary = self
            .temporary_path
            .as_deref()
            .and_then(Path::file_name)
            .ok_or_else(invalid)?;
        owned_path(&directory, temporary, file)?;
        if expected.is_some() {
            at::renameat(&directory, temporary, &directory, target)?;
        } else {
            publish_new(&directory, temporary, target)?;
        }
        // Durability only: the rename has already published the file.
        let _ = directory.sync_all();
        self.temporary.take();
        self.temporary_path.take();
        Ok(())
    }

    pub fn discard(&mut self) -> io::Result<()> {
        if let (Some(file), Some(path)) = (self.temporary.as_ref(), self.temporary_path.as_ref()) {
            // A missing name, or one that now belongs to another file, is
            // never deleted on this capability's behalf.
            if let (Ok(directory), Some(name)) = (self.anchor.checked_parent(), path.file_name()) {
                if owned_path(&directory, name, file).is_ok() {
                    at::unlinkat(directory, name, AtFlags::empty())?;
                }
            }
        }
        self.temporary.take();
        self.temporary_path.take();
        Ok(())
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        if !self.preserve_on_drop {
            let _ = self.discard();
        }
    }
}

fn walk_directories(
    mut directory: File,
    components: &[OsString],
    create: bool,
) -> io::Result<File> {
    for name in components {
        directory = match open_subdirectory(&directory, name) {
            Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                match at::mkdirat(&directory, name, Mode::RWXU) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(error.into()),
                }
                open_subdirectory(&directory, name)?
            }
            result => result?,
        };
    }
    Ok(directory)
}

pub fn prepare_destination_folder(root: &Path, directories: &[PathBuf]) -> io::Result<()> {
    SelectedDirectory::open(root)?.prepare(directories)
}

/// Scan the selected folder once. Capturing versions with fstatat avoids
/// opening each file until a transfer worker reads it.
pub fn scan_folder(path: &Path) -> io::Result<super::FolderContents> {
    let selected = SelectedDirectory::open(path)?;
    let root = selected.checked_root()?;
    let mut files = Vec::new();
    let mut directories = Vec::new();
    let mut pending = vec![(PathBuf::new(), selected.identity)];
    while let Some((relative, expected)) = pending.pop() {
        let directory = if relative.as_os_str().is_empty() {
            root.try_clone()?
        } else {
            let components = valid_relative_components(&relative)?;
            walk_directories(root.try_clone()?, &components, false)?
        };
        if identity(&at::fstat(&directory)?) != expected {
            return Err(conflict());
        }
        let base = selected.root.join(&relative);
        for entry in Dir::read_from(&directory)? {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            // Finder view state is not user content and should not reach servers.
            if cfg!(target_os = "macos") && name == ".DS_Store" {
                continue;
            }
            if files.len() + directories.len() >= MAX_TREE_ENTRIES - 1 {
                return Err(invalid());
            }
            if !name.to_str().is_some_and(valid_name) {
                return Err(invalid());
            }
            let child = relative.join(name);
            let stat = at::statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => {
                    pending.push((child.clone(), identity(&stat)));
                    directories.push(child);
                }
                FileType::RegularFile => {
                    valid_relative_components(&child)?;
                    let anchor = AnchoredPath {
                        path: base.join(name),
                        parent_identity: expected,
                    };
                    let source =
                        Source::from_snapshot(anchor, version_of(&stat, FileType::RegularFile)?);
                    files.push((child, source));
                }
                _ => return Err(invalid()),
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    Ok((directories, files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn temp() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        // macOS temp paths live under the /var -> /private/var link.
        let root = fs::canonicalize(directory.path()).unwrap();
        (directory, root)
    }

    #[test]
    fn names_reject_separators_controls_and_dot_entries() {
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "line\nbreak",
            "nul\0",
            &"x".repeat(256),
        ] {
            assert!(!valid_name(name), "{name:?}");
        }
        for name in [
            "report.txt",
            "CON",
            "trailing.",
            "colon:name",
            "中文.log",
            ".hidden",
        ] {
            assert!(valid_name(name), "{name:?}");
        }
    }

    #[test]
    fn new_file_commit_never_replaces_and_leaves_no_temporary() {
        let (_guard, root) = temp();
        let target = root.join("download.bin");
        let mut destination = Destination::select(&target).unwrap();
        assert_eq!(destination.snapshot().unwrap(), None);
        destination.begin(".download.partial").unwrap();
        destination.write(0, b"hello").unwrap();
        destination.write(5, b" world").unwrap();
        assert_eq!(destination.partial_size().unwrap(), Some(11));
        assert_eq!(destination.read(0, 5).unwrap(), b"hello");
        fs::write(&target, b"raced").unwrap();
        assert_eq!(
            destination.commit(None).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&target).unwrap(), b"raced");
        drop(destination);
        assert!(!root.join(".download.partial").exists());

        fs::remove_file(&target).unwrap();
        let mut destination = Destination::select(&target).unwrap();
        destination.begin(".download.partial").unwrap();
        destination.write(0, b"fresh").unwrap();
        destination.commit(None).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"fresh");
        assert!(!root.join(".download.partial").exists());
    }

    #[test]
    fn replace_requires_the_expected_version() {
        let (_guard, root) = temp();
        let target = root.join("settings.json");
        fs::write(&target, b"old").unwrap();
        let mut destination = Destination::select(&target).unwrap();
        let expected = destination.snapshot().unwrap();
        assert!(expected.is_some());
        destination.begin(".settings.partial").unwrap();
        destination.write(0, b"new").unwrap();
        fs::write(&target, b"changed elsewhere").unwrap();
        assert!(destination.commit(expected).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"changed elsewhere");

        let expected = destination.snapshot().unwrap();
        destination.commit(expected).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn links_and_special_files_are_never_followed() {
        let (_guard, root) = temp();
        let real = root.join("real.txt");
        fs::write(&real, b"secret").unwrap();
        let link = root.join("link.txt");
        symlink(&real, &link).unwrap();
        assert_eq!(
            Source::open(&link).err().map(|error| error.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
        assert!(Destination::select(&link).is_err());

        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let selected = root.join("selected");
        fs::create_dir(&selected).unwrap();
        symlink(&elsewhere, selected.join("escape")).unwrap();
        assert!(Destination::select_under_folder(&selected, Path::new("escape/file.txt")).is_err());
        assert!(scan_folder(&selected).is_err());
        assert!(fs::read_dir(&elsewhere).unwrap().next().is_none());
    }

    #[test]
    fn ancestor_links_are_resolved_once_for_picker_paths() {
        let (_guard, root) = temp();
        fs::create_dir(root.join("real")).unwrap();
        fs::write(root.join("real/file.txt"), b"data").unwrap();
        symlink(root.join("real"), root.join("alias")).unwrap();
        let source = Source::open(&root.join("alias/file.txt")).unwrap();
        assert_eq!(source.read(0, 16).unwrap(), b"data");
        let destination = Destination::select(&root.join("alias/out.txt")).unwrap();
        assert_eq!(destination.target_path(), root.join("real/out.txt"));
    }

    #[test]
    fn sources_detect_changes_and_hold_no_idle_descriptor() {
        let (_guard, root) = temp();
        let path = root.join("upload.txt");
        fs::write(&path, b"0123456789").unwrap();
        let source = Source::open(&path).unwrap();
        assert!(source.file.lock().unwrap().is_none());
        assert_eq!(source.size(), 10);
        assert_eq!(source.read(4, 4).unwrap(), b"4567");
        assert!(source.read(0, MAX_IO + 1).is_err());
        source.verify().unwrap();
        assert!(source.file.lock().unwrap().is_none());

        // Same size, new content in place: the reopen must see the new mtime.
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, b"9876543210").unwrap();
        assert_eq!(
            source.read(0, 4).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(source.verify().is_err());

        // An atomic replacement is a different inode even with equal metadata.
        let other = root.join("other.txt");
        fs::write(&other, b"0123456789").unwrap();
        let source = Source::open(&other).unwrap();
        let swapped = root.join("swapped.txt");
        fs::write(&swapped, b"0123456789").unwrap();
        fs::rename(&swapped, &other).unwrap();
        assert!(source.read(0, 4).is_err());
    }

    #[test]
    fn replaced_temporary_is_neither_published_nor_deleted() {
        let (_guard, root) = temp();
        let target = root.join("file.txt");
        let mut destination = Destination::select(&target).unwrap();
        destination.begin(".file.partial").unwrap();
        destination.write(0, b"ours").unwrap();
        let temporary = root.join(".file.partial");
        fs::remove_file(&temporary).unwrap();
        fs::write(&temporary, b"theirs").unwrap();
        assert!(destination.commit(None).is_err());
        assert!(!target.exists());
        drop(destination);
        assert_eq!(fs::read(&temporary).unwrap(), b"theirs");
    }

    #[test]
    fn preserved_partials_can_be_recovered_by_identity() {
        let (_guard, root) = temp();
        let target = root.join("big.iso");
        let mut first = Destination::select(&target).unwrap();
        first.begin(".big.partial").unwrap();
        first.write(0, b"abcdef").unwrap();
        let path = first.partial_path().unwrap();
        let identity = first.partial_identity().unwrap().unwrap();
        first.preserve_partial();
        drop(first);
        assert!(path.exists());

        let mut wrong = Destination::select(&target).unwrap();
        assert!(wrong.recover_partial(&path, identity ^ 1, 0, 100).is_err());
        assert!(wrong.recover_partial(&path, identity, 10, 100).is_err());
        let mut second = Destination::select(&target).unwrap();
        second.recover_partial(&path, identity, 6, 6).unwrap();
        second.write(6, b"gh").unwrap();
        second.commit(None).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"abcdefgh");
        assert!(!path.exists());
    }

    #[test]
    fn folder_enumeration_is_sorted_bounded_and_creates_directories() {
        let (_guard, root) = temp();
        let selected = root.join("project");
        fs::create_dir_all(selected.join("src/nested")).unwrap();
        fs::create_dir(selected.join("empty")).unwrap();
        fs::write(selected.join("README"), b"r").unwrap();
        fs::write(selected.join("src/main.rs"), b"m").unwrap();
        fs::write(selected.join("src/nested/a.txt"), b"a").unwrap();
        let (directories, files) = scan_folder(&selected).unwrap();
        let names: Vec<_> = files.iter().map(|(path, _)| path.clone()).collect();
        assert_eq!(
            names,
            [
                PathBuf::from("README"),
                PathBuf::from("src/main.rs"),
                PathBuf::from("src/nested/a.txt"),
            ]
        );
        assert_eq!(files[2].1.read(0, 1).unwrap(), b"a");
        assert_eq!(
            directories,
            [
                PathBuf::from("empty"),
                PathBuf::from("src"),
                PathBuf::from("src/nested"),
            ]
        );

        let output = root.join("download");
        fs::create_dir(&output).unwrap();
        prepare_destination_folder(&output, &[PathBuf::from("a"), PathBuf::from("a/b")]).unwrap();
        assert!(output.join("a/b").is_dir());
        assert!(prepare_destination_folder(&output, &[PathBuf::from("../x")]).is_err());
        let selected = SelectedDirectory::open(&output).unwrap();
        assert_eq!(selected.snapshot(Path::new("a/missing.txt")).unwrap(), None);
        let mut destination =
            Destination::select_under_folder(&output, Path::new("c/d/file.txt")).unwrap();
        destination.begin(".file.partial").unwrap();
        destination.write(0, b"x").unwrap();
        destination.commit(None).unwrap();
        assert_eq!(fs::read(output.join("c/d/file.txt")).unwrap(), b"x");
        assert!(selected
            .snapshot(Path::new("c/d/file.txt"))
            .unwrap()
            .is_some());
    }

    #[test]
    fn check_then_rename_publish_never_replaces_an_existing_target() {
        let (_guard, root) = temp();
        let parent = File::from(at::open(&root, DIRECTORY_FLAGS, Mode::empty()).unwrap());
        fs::write(root.join(".partial"), b"new").unwrap();
        fs::write(root.join("taken"), b"old").unwrap();
        assert_eq!(
            publish_after_check(&parent, OsStr::new(".partial"), OsStr::new("taken"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(root.join("taken")).unwrap(), b"old");
        publish_after_check(&parent, OsStr::new(".partial"), OsStr::new("free")).unwrap();
        assert_eq!(fs::read(root.join("free")).unwrap(), b"new");
        assert!(!root.join(".partial").exists());
        assert!(lacks_exclusive_publish(rustix::io::Errno::NOTSUP));
        assert!(!lacks_exclusive_publish(rustix::io::Errno::EXIST));
    }

    #[test]
    fn open_file_limit_only_ever_rises() {
        use rustix::process::{getrlimit, Resource};
        let before = getrlimit(Resource::Nofile).current;
        raise_open_file_limit().unwrap();
        raise_open_file_limit().unwrap();
        let after = getrlimit(Resource::Nofile).current;
        assert!(after.is_none() || after >= before);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn folder_scans_skip_finder_metadata_but_keep_other_dotfiles() {
        let (_guard, root) = temp();
        fs::write(root.join(".DS_Store"), b"view state").unwrap();
        fs::write(root.join(".env"), b"kept").unwrap();
        let (_, files) = scan_folder(&root).unwrap();
        let names: Vec<_> = files.iter().map(|(path, _)| path.clone()).collect();
        assert_eq!(names, [PathBuf::from(".env")]);
    }

    #[test]
    fn relative_and_parent_components_are_rejected() {
        assert!(Source::open(Path::new("relative.txt")).is_err());
        assert!(Destination::select(Path::new("/tmp/../etc/passwd")).is_err());
        assert!(Destination::select(Path::new("/")).is_err());
        let (_guard, root) = temp();
        assert!(Destination::select_under_folder(&root, Path::new("/abs")).is_err());
        assert!(Destination::select_under_folder(&root, Path::new("")).is_err());
    }
}
