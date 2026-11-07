//! Picker-selected regular files. Held ancestor handles deny delete/rename,
//! and final opens reject reparse points. Temporary files are renamed/deleted
//! by handle, never by reopening a previously checked temporary pathname.
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE},
    Storage::FileSystem::{
        FileDispositionInfo, FileRenameInfo, GetFileInformationByHandle,
        SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
};

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

pub fn valid_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    !name.is_empty()
        && name.encode_utf16().count() <= 255
        && !name.ends_with(['.', ' '])
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && !matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}
pub(crate) fn version(file: &File, directory: bool) -> io::Result<Version> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file owns a live handle and info is a writable, correctly sized structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
    {
        return Err(invalid());
    }
    Ok(Version {
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
        modified: (u64::from(info.ftLastWriteTime.dwHighDateTime) << 32)
            | u64::from(info.ftLastWriteTime.dwLowDateTime),
        identity: (u128::from(info.dwVolumeSerialNumber) << 64)
            | (u128::from(info.nFileIndexHigh) << 32)
            | u128::from(info.nFileIndexLow),
        attributes: info.dwFileAttributes,
    })
}
pub(crate) struct AnchoredPath {
    path: PathBuf,
    _pins: Vec<File>,
}
impl AnchoredPath {
    pub(crate) fn new(path: &Path) -> io::Result<Self> {
        if path.as_os_str().encode_wide().count() > 32_767 {
            return Err(invalid());
        }
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(invalid());
        };
        if !matches!(prefix.kind(), Prefix::Disk(_))
            || components.next() != Some(Component::RootDir)
        {
            return Err(invalid());
        }
        let rest: Vec<_> = components.collect();
        if rest.is_empty() || rest.len() > 64 {
            return Err(invalid());
        }
        for component in &rest {
            let Component::Normal(name) = component else {
                return Err(invalid());
            };
            if !name.to_str().is_some_and(valid_name) {
                return Err(invalid());
            }
        }
        // Pin from the volume root downward before opening the next component.
        // Replacing any ancestor now fails with a Windows sharing violation.
        let mut current = PathBuf::from(prefix.as_os_str());
        current.push("\\");
        let mut pins = Vec::new();
        for component in std::iter::once(None).chain(rest[..rest.len() - 1].iter().map(Some)) {
            if let Some(component) = component {
                current.push(component.as_os_str());
            }
            let file = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&current)?;
            version(&file, true)?;
            pins.push(file);
        }
        current.push(rest[rest.len() - 1].as_os_str());
        Ok(Self {
            path: current,
            _pins: pins,
        })
    }
    fn snapshot(&self) -> io::Result<Option<Version>> {
        match OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(&self.path)
        {
            Ok(file) => version(&file, file.metadata()?.is_dir()).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

pub struct Source {
    anchor: AnchoredPath,
    file: File,
    baseline: Version,
}

/// A bounded, reparse-point-free snapshot of regular files beneath a selected
/// directory. Each returned Source keeps its ancestor handles pinned, so the
/// source remains tied to the directory tree selected by the user.
pub fn enumerate_folder(path: &Path) -> io::Result<Vec<(PathBuf, Source)>> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let root = path.to_owned();
    let metadata = std::fs::symlink_metadata(&root)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid());
    }
    let mut files = Vec::new();
    let mut pending = vec![(root.clone(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth >= 64 {
            return Err(invalid());
        }
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(invalid());
            }
            if metadata.is_dir() {
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                if files.len() >= 4096 {
                    return Err(invalid());
                }
                let relative = path.strip_prefix(&root).map_err(|_| invalid())?.to_owned();
                if relative.components().any(|component| {
                    !matches!(component, Component::Normal(name) if name.to_str().is_some_and(valid_name))
                }) {
                    return Err(invalid());
                }
                files.push((relative, Source::open(&path)?));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

/// Return the regular directory entries beneath a selected folder.
///
/// The selected root itself is deliberately omitted. Callers can create it
/// at the remote destination and then create these entries in parent-before-
/// child order so an empty local folder is preserved as well.
pub fn enumerate_folder_directories(path: &Path) -> io::Result<Vec<PathBuf>> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let root = path.to_owned();
    let metadata = std::fs::symlink_metadata(&root)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid());
    }
    let mut directories = Vec::new();
    let mut pending = vec![(root.clone(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth >= 64 {
            return Err(invalid());
        }
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let child = entry.path();
            let metadata = std::fs::symlink_metadata(&child)?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(invalid());
            }
            if metadata.is_dir() {
                if directories.len() >= 4096 {
                    return Err(invalid());
                }
                let relative = child.strip_prefix(&root).map_err(|_| invalid())?.to_owned();
                if relative.components().any(|component| {
                    !matches!(component, Component::Normal(name) if name.to_str().is_some_and(valid_name))
                }) {
                    return Err(invalid());
                }
                directories.push(relative);
                pending.push((child, depth + 1));
            } else if !metadata.is_file() {
                return Err(invalid());
            }
        }
    }
    directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    Ok(directories)
}
impl Source {
    pub fn open(path: &Path) -> io::Result<Self> {
        let anchor = AnchoredPath::new(path)?;
        // Deny both writes and deletion for this source capability's lifetime.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(&anchor.path)?;
        let baseline = version(&file, false)?;
        Ok(Self {
            anchor,
            file,
            baseline,
        })
    }
    pub fn name(&self) -> io::Result<String> {
        self.anchor
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .ok_or_else(invalid)
    }
    pub fn size(&self) -> u64 {
        self.baseline.size
    }
    pub fn read(&self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        if length > 32768 {
            return Err(invalid());
        }
        let mut data = vec![0; length];
        let count = self.file.seek_read(&mut data, offset)?;
        data.truncate(count);
        Ok(data)
    }
    pub fn verify(&self) -> io::Result<()> {
        if version(&self.file, false)? == self.baseline {
            Ok(())
        } else {
            Err(conflict())
        }
    }
}

pub struct Destination {
    anchor: AnchoredPath,
    temporary: Option<File>,
    temporary_path: Option<PathBuf>,
    preserve_on_drop: bool,
}

/// Pins a native picker-selected folder without creating anything under it.
pub struct SelectedDirectory {
    root: PathBuf,
    _anchor: AnchoredPath,
}
impl SelectedDirectory {
    pub fn open(root: &Path) -> io::Result<Self> {
        let anchor = AnchoredPath::new(&root.join(".capstan-anchor"))?;
        Ok(Self {
            root: root.to_owned(),
            _anchor: anchor,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn snapshot(&self, relative: &Path) -> io::Result<Option<Version>> {
        valid_relative_components(relative)?;
        match AnchoredPath::new(&self.root.join(relative)) {
            Ok(anchor) => anchor.snapshot(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}
impl Destination {
    pub fn select(path: &Path) -> io::Result<Self> {
        let anchor = AnchoredPath::new(path)?;
        anchor.snapshot()?;
        Ok(Self {
            anchor,
            temporary: None,
            temporary_path: None,
            preserve_on_drop: false,
        })
    }
    /// Create a destination capability below a picker-selected directory.
    /// Every parent component is checked for reparse points before it is
    /// created or reused; the final file itself may already exist so the
    /// transfer conflict workflow can decide whether to replace it.
    pub fn select_under_folder(root: &Path, relative: &Path) -> io::Result<Self> {
        let components = valid_relative_components(relative)?;
        if components.is_empty() {
            return Err(invalid());
        }
        ensure_directory_components(root, &components[..components.len() - 1])?;
        Self::select(&root.join(relative))
    }
    pub fn name(&self) -> io::Result<String> {
        self.anchor
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .ok_or_else(invalid)
    }
    /// Return the target chosen by the native picker for recovery metadata.
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
        let path = self.anchor.path.with_file_name(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .share_mode(0)
            .create_new(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)?;
        version(&file, false)?;
        self.temporary_path = Some(path);
        self.temporary = Some(file);
        Ok(())
    }
    pub fn partial_size(&self) -> io::Result<Option<u64>> {
        self.temporary
            .as_ref()
            .map(|file| version(file, false).map(|value| value.size))
            .transpose()
    }
    /// Return the exact temporary path owned by this destination. The value
    /// is used only by the Rust recovery ledger and is never a WebView path
    /// capability.
    pub fn partial_path(&self) -> Option<PathBuf> {
        self.temporary_path.clone()
    }
    pub fn partial_identity(&self) -> io::Result<Option<u128>> {
        self.temporary
            .as_ref()
            .map(|file| version(file, false).map(|v| v.identity))
            .transpose()
    }
    pub fn preserve_partial(&mut self) {
        self.preserve_on_drop = true;
    }
    /// Reattach only a recorded sibling of a newly picker-selected target.
    /// The exclusive handle pins the verified file for all subsequent I/O.
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
        let anchor = AnchoredPath::new(path)?;
        let file = OpenOptions::new()
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&anchor.path)?;
        let actual = version(&file, false)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file and the output structure remain live through this call.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if actual.identity != identity
            || actual.size < minimum
            || actual.size > maximum
            || info.nNumberOfLinks != 1
        {
            return Err(conflict());
        }
        self.temporary = Some(file);
        self.temporary_path = Some(anchor.path);
        // An abandoned picker/grant must not delete a pre-existing partial.
        self.preserve_on_drop = true;
        Ok(())
    }
    pub fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        if data.len() > 32768 {
            return Err(invalid());
        }
        let file = self.temporary.as_ref().ok_or_else(invalid)?;
        let mut written = 0;
        while written < data.len() {
            let count = file.seek_write(&data[written..], offset + written as u64)?;
            if count == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero));
            }
            written += count;
        }
        Ok(())
    }
    pub fn read(&self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        if length > 32768 {
            return Err(invalid());
        }
        let mut data = vec![0; length];
        let count = self
            .temporary
            .as_ref()
            .ok_or_else(invalid)?
            .seek_read(&mut data, offset)?;
        data.truncate(count);
        Ok(data)
    }
    pub fn sync(&self) -> io::Result<()> {
        self.temporary.as_ref().ok_or_else(invalid)?.sync_all()
    }
    pub fn commit(&mut self, expected: Option<Version>) -> io::Result<()> {
        if expected.is_some_and(|version| version.attributes & FILE_ATTRIBUTE_DIRECTORY != 0) {
            return Err(conflict());
        }
        self.sync()?;
        if self.snapshot()? != expected {
            return Err(conflict());
        }
        let file = self.temporary.as_ref().ok_or_else(invalid)?;
        let target: Vec<u16> = self.anchor.path.as_os_str().encode_wide().collect();
        let bytes = target.len() * 2;
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        // SetFileInformationByHandle accepts a DOS path here. Keep an explicit
        // wide NUL for that path conversion even when no alignment padding is
        // left after FileName. FileNameLength excludes the terminator.
        let size = offset + bytes + std::mem::size_of::<u16>();
        let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
        let rename = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        // SAFETY: usize storage has FILE_RENAME_INFO alignment and space for its
        // UTF-16 flexible array. file and all directory pins remain live. The
        // source is the held temporary handle, not a replaceable pathname.
        unsafe {
            (*rename).Anonymous.ReplaceIfExists = expected.is_some();
            (*rename).RootDirectory = std::ptr::null_mut();
            (*rename).FileNameLength = bytes as u32;
            std::ptr::copy_nonoverlapping(
                target.as_ptr(),
                (*rename).FileName.as_mut_ptr(),
                target.len(),
            );
            if SetFileInformationByHandle(
                file.as_raw_handle(),
                FileRenameInfo,
                rename.cast(),
                size as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        self.temporary.take();
        self.temporary_path.take();
        Ok(())
    }
    pub fn discard(&mut self) -> io::Result<()> {
        if let Some(file) = self.temporary.as_ref() {
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: the owned handle and correctly sized structure stay live.
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
            self.temporary.take();
            self.temporary_path.take();
        }
        Ok(())
    }
}
fn valid_relative_components(path: &Path) -> io::Result<Vec<std::ffi::OsString>> {
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
        components.push(std::ffi::OsString::from(name));
    }
    if components.is_empty() || components.len() > 64 {
        return Err(invalid());
    }
    Ok(components)
}

fn ensure_directory_components(root: &Path, components: &[std::ffi::OsString]) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !root.is_absolute()
        || !metadata.is_dir()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(invalid());
    }
    let mut current = root.to_owned();
    for component in components {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.is_dir()
                    && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 => {}
            Ok(_) => return Err(invalid()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)?;
                let metadata = std::fs::symlink_metadata(&current)?;
                if !metadata.is_dir()
                    || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                {
                    return Err(invalid());
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Ensure every directory in a remote tree has a local counterpart.
pub fn prepare_destination_folder(root: &Path, directories: &[PathBuf]) -> io::Result<()> {
    if directories.len() > 4096 {
        return Err(invalid());
    }
    for directory in directories {
        let components = valid_relative_components(directory)?;
        ensure_directory_components(root, &components)?;
    }
    Ok(())
}

impl Drop for Destination {
    fn drop(&mut self) {
        if !self.preserve_on_drop {
            let _ = self.discard();
        }
    }
}

/// All native text is provided by the caller's selected locale. No remote
/// filename is interpolated into a dialog filter, shell command or default path.
pub fn pick_file(
    title: &str,
    filter: &str,
    parent: isize,
    save: bool,
) -> io::Result<Option<PathBuf>> {
    pick_file_with_pattern(title, filter, "*.*", parent, save)
}

/// Accepts a static common-dialog pattern, such as "*.json;*.itermcolors".
pub fn pick_file_with_pattern(
    title: &str,
    filter: &str,
    pattern: &str,
    parent: isize,
    save: bool,
) -> io::Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST,
        OFN_NOCHANGEDIR, OFN_NODEREFERENCELINKS, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let filter: Vec<u16> = format!("{filter}\0{pattern}\0\0").encode_utf16().collect();
    let mut file = vec![0_u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: parent as _,
        lpstrTitle: title.as_ptr(),
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        Flags: OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_NODEREFERENCELINKS
            | if save { 0 } else { OFN_FILEMUSTEXIST },
        ..Default::default()
    };
    // SAFETY: all dialog buffers remain live through the synchronous call.
    if unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    } == 0
    {
        // SAFETY: this only reads the calling thread's common-dialog error.
        if unsafe { CommDlgExtendedError() } == 0 {
            return Ok(None);
        }
        return Err(io::Error::other("native transfer picker failed"));
    }
    let end = file
        .iter()
        .position(|value| *value == 0)
        .ok_or_else(invalid)?;
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
        &file[..end],
    ))))
}

/// Pick a directory without exposing its path to the WebView. The caller must
/// immediately convert the result into a bounded native capability.
pub fn pick_folder(title: &str, parent: isize) -> io::Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{SHBrowseForFolderW, SHGetPathFromIDListW, BROWSEINFOW},
    };
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let mut display = vec![0_u16; 32768];
    let browse = BROWSEINFOW {
        hwndOwner: parent as _,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: 0x0001 | 0x0040, // BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    // SAFETY: all buffers live across the synchronous dialog call.
    let pidl = unsafe { SHBrowseForFolderW(&browse) };
    if pidl.is_null() {
        return Ok(None);
    }
    let mut path = vec![0_u16; 32768];
    // SAFETY: pidl is owned by the shell call and path is writable.
    let result = unsafe { SHGetPathFromIDListW(pidl, path.as_mut_ptr()) };
    // SAFETY: the shell allocates the returned PIDL with the task allocator.
    unsafe { CoTaskMemFree(pidl.cast()) };
    if result == 0 {
        return Err(invalid());
    }
    let end = path
        .iter()
        .position(|value| *value == 0)
        .ok_or_else(invalid)?;
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
        &path[..end],
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_reject_windows_aliases_and_escape() {
        for name in [
            "../x",
            "x:y",
            "\\device",
            "NUL.txt",
            "aux .txt",
            "COM1",
            "LPT¹.log",
            "a.",
            "a ",
            "CONIN$",
            "",
        ] {
            assert!(!valid_name(name), "{name}");
        }
        assert!(valid_name("中文 space.txt"));
    }
    #[test]
    fn source_pins_ancestors_and_denies_writes() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("parent");
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("source");
        std::fs::write(&path, b"safe").unwrap();
        let source = Source::open(&path).unwrap();
        assert_eq!(source.read(0, 32).unwrap(), b"safe");
        assert!(std::fs::rename(&directory, root.path().join("moved")).is_err());
        assert!(std::fs::write(&path, b"changed").is_err());
        source.verify().unwrap();
        drop(source);
        std::fs::rename(&directory, root.path().join("moved")).unwrap();
    }
    #[test]
    fn atomic_commit_and_changed_target_preserve_original() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("中文 target.txt");
        std::fs::write(&path, b"original").unwrap();
        let mut destination = Destination::select(&path).unwrap();
        let baseline = destination.snapshot().unwrap();
        destination.begin(".capstan-test.partial").unwrap();
        assert_eq!(
            destination.partial_path(),
            Some(root.path().join(".capstan-test.partial"))
        );
        destination.write(0, b"new bytes").unwrap();
        std::fs::write(&path, b"concurrent change").unwrap();
        assert_eq!(
            destination.commit(baseline).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"concurrent change");
        let baseline = destination.snapshot().unwrap();
        destination.commit(baseline).unwrap();
        assert_eq!(destination.partial_path(), None);
        assert_eq!(std::fs::read(&path).unwrap(), b"new bytes");
        assert!(!root.path().join(".capstan-test.partial").exists());
    }
    #[test]
    fn equal_size_and_time_do_not_authorize_a_replaced_file_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("target");
        std::fs::write(&path, b"original").unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut destination = Destination::select(&path).unwrap();
        let baseline = destination.snapshot().unwrap().unwrap();
        destination.begin(".capstan-test.partial").unwrap();
        destination.write(0, b"our data").unwrap();
        let replacement = root.path().join("replacement");
        std::fs::write(&replacement, b"external").unwrap();
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        std::fs::rename(&path, root.path().join("previous")).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let current = destination.snapshot().unwrap().unwrap();
        assert_eq!(current.size, baseline.size);
        assert_eq!(current.modified, baseline.modified);
        assert_ne!(current.identity, baseline.identity);
        assert_eq!(
            destination.commit(Some(baseline)).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"external");
        destination.discard().unwrap();
    }
    #[test]
    fn new_target_race_and_discard_do_not_overwrite_or_leak() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("target");
        let mut destination = Destination::select(&path).unwrap();
        destination.begin(".capstan-test.partial").unwrap();
        destination.write(0, b"uncommitted").unwrap();
        assert!(std::fs::remove_file(root.path().join(".capstan-test.partial")).is_err());
        std::fs::write(&path, b"appeared").unwrap();
        assert!(destination.commit(None).is_err());
        destination.discard().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"appeared");
        assert!(!root.path().join(".capstan-test.partial").exists());
    }
    #[test]
    fn folder_enumeration_is_bounded_and_preserves_relative_names() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("中文.txt"), b"one").unwrap();
        std::fs::write(root.path().join("nested").join("two.txt"), b"two").unwrap();
        let files = enumerate_folder(root.path()).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|(path, _)| path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, vec!["nested/two.txt", "中文.txt"]);
        assert_eq!(files[0].1.read(0, 8).unwrap(), b"two");
    }
    #[test]
    fn folder_directory_enumeration_keeps_empty_and_parent_before_child() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("empty")).unwrap();
        std::fs::create_dir_all(root.path().join("nested").join("deeper")).unwrap();
        std::fs::write(root.path().join("nested").join("deeper").join("file"), b"x").unwrap();
        let directories = enumerate_folder_directories(root.path()).unwrap();
        let names: Vec<_> = directories
            .iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, vec!["empty", "nested", "nested/deeper"]);
    }
    #[test]
    fn publication_preserves_unicode_and_hidden_files_at_every_name_alignment() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("中文 space");
        std::fs::create_dir(&folder).unwrap();
        for length in 0..8 {
            let target = folder.join(format!(".hidden-{}.bin", "x".repeat(length)));
            let mut selected = Destination::select(&target).unwrap();
            selected.begin(".capstan-alignment.partial").unwrap();
            let bytes = if length % 2 == 0 {
                b"".as_slice()
            } else {
                b"payload"
            };
            selected.write(0, bytes).unwrap();
            selected.sync().unwrap();
            assert_eq!(selected.partial_size().unwrap(), Some(bytes.len() as u64));
            selected.commit(None).unwrap();
            assert_eq!(std::fs::read(target).unwrap(), bytes);
            assert!(!folder.join(".capstan-alignment.partial").exists());
        }
    }
    #[test]
    fn folder_destination_creates_empty_directories_and_keeps_file_capability_anchored() {
        let root = tempfile::tempdir().unwrap();
        prepare_destination_folder(
            root.path(),
            &[PathBuf::from("remote"), PathBuf::from("remote/empty")],
        )
        .unwrap();
        assert!(root.path().join("remote").join("empty").is_dir());
        let path = PathBuf::from("remote/nested/file.txt");
        let mut destination = Destination::select_under_folder(root.path(), &path).unwrap();
        destination.begin(".capstan-test.partial").unwrap();
        destination.write(0, b"file").unwrap();
        destination.commit(None).unwrap();
        assert_eq!(std::fs::read(root.path().join(&path)).unwrap(), b"file");
    }
    #[test]
    fn junction_ancestor_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("real");
        let link = root.path().join("junction");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file"), b"outside").unwrap();
        let result = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(Source::open(&link.join("file")).is_err());
        assert!(Destination::select(&link.join("file")).is_err());
        std::fs::remove_dir(&link).unwrap();
    }
    #[test]
    fn recovered_partial_rejects_replacement_hard_links_and_size_changes() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("final.bin");
        let partial = root.path().join(".capstan-test.partial");
        let mut first = Destination::select(&target).unwrap();
        first.begin(".capstan-test.partial").unwrap();
        first.write(0, b"prefix").unwrap();
        let identity = first.partial_identity().unwrap().unwrap();
        first.preserve_partial();
        drop(first);
        let mut selected = Destination::select(&target).unwrap();
        assert!(selected.recover_partial(&partial, identity, 7, 20).is_err());
        assert!(selected.recover_partial(&partial, identity, 0, 5).is_err());
        let alias = root.path().join("alias");
        std::fs::hard_link(&partial, &alias).unwrap();
        assert!(selected.recover_partial(&partial, identity, 0, 20).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::rename(&partial, &alias).unwrap();
        std::fs::write(&partial, b"prefix").unwrap();
        assert!(selected.recover_partial(&partial, identity, 6, 20).is_err());
        assert_eq!(std::fs::read(&partial).unwrap(), b"prefix");
        std::fs::remove_file(&partial).unwrap();
        std::fs::rename(&alias, &partial).unwrap();
        selected.recover_partial(&partial, identity, 6, 20).unwrap();
        assert!(std::fs::rename(&partial, &alias).is_err());
        selected.discard().unwrap();
        assert!(!partial.exists());
    }
}
