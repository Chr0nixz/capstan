//! Private, explicitly retained Explorer exports. Drag completion never deletes data.
use crate::transfer::{valid_name, version, AnchoredPath, Source};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE},
    Storage::FileSystem::{
        FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
        BY_HANDLE_FILE_INFORMATION, DELETE, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
};

const MARKER: &str = ".capstan-export";
const MAGIC: &str = "CAPSTAN_EXPORT_V1\n";
pub struct ExportStage {
    root: PathBuf,
    root_identity: u128,
    directories: Vec<PathBuf>,
    files: Vec<PathBuf>,
    partials: Vec<PathBuf>,
    marker: Option<File>,
    anchor: Option<AnchoredPath>,
    _parent: AnchoredPath,
    pins: Vec<Source>,
    directory_pins: Vec<AnchoredPath>,
}
struct UncommittedStage(Option<ExportStage>);
impl Drop for UncommittedStage {
    fn drop(&mut self) {
        if let Some(stage) = &mut self.0 {
            // The same bounded ownership/identity checks apply even before
            // the constructor has written a complete marker.
            let _ = stage.cleanup();
        }
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid export stage")
}
fn relative(value: &str) -> io::Result<PathBuf> {
    if value.len() > 4096
        || value.split('/').count() > 64
        || value.split('/').any(|part| !valid_name(part))
        || value.split('/').next() == Some(MARKER)
    {
        return Err(invalid());
    }
    Ok(value.split('/').collect())
}
fn validate(directories: &[String], files: &[String]) -> io::Result<()> {
    if directories.len() + files.len() > 4096 {
        return Err(invalid());
    }
    let mut names = HashSet::new();
    for name in directories.iter().chain(files) {
        relative(name)?;
        if !names.insert(name.to_uppercase()) {
            return Err(invalid());
        }
    }
    Ok(())
}
impl ExportStage {
    pub fn prepare(
        base: &Path,
        id: &str,
        directories: &[String],
        files: &[String],
    ) -> io::Result<Self> {
        Self::prepare_with_checkpoint(base, id, directories, files, |_| Ok(()))
    }
    fn prepare_with_checkpoint(
        base: &Path,
        id: &str,
        directories: &[String],
        files: &[String],
        mut checkpoint: impl FnMut(&'static str) -> io::Result<()>,
    ) -> io::Result<Self> {
        if id.len() != 36 || !id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-') {
            return Err(invalid());
        }
        validate(directories, files)?;
        let prepared_directories = directories
            .iter()
            .map(|name| relative(name))
            .collect::<io::Result<_>>()?;
        let prepared_files = files
            .iter()
            .map(|name| relative(name))
            .collect::<io::Result<_>>()?;
        fs::create_dir_all(base)?;
        let root = base.join(id);
        let parent = AnchoredPath::new(&root)?;
        fs::create_dir(&root)?;
        // Without an original identity no pathname is safe to remove. Once
        // captured, guard every subsequent fallible initialization step.
        let root_identity = directory_identity(&root)?;
        let mut pending = UncommittedStage(Some(Self {
            root,
            root_identity,
            directories: prepared_directories,
            files: prepared_files,
            marker: None,
            anchor: None,
            _parent: parent,
            pins: Vec::new(),
            directory_pins: Vec::new(),
            partials: Vec::new(),
        }));
        let stage = pending.0.as_mut().ok_or_else(invalid)?;
        checkpoint("root_identity")?;
        crate::editor::restrict_directory(&stage.root)?;
        checkpoint("restricted")?;
        stage.anchor = Some(AnchoredPath::new(&stage.root.join(MARKER))?);
        stage.marker = Some(open_marker(&stage.root.join(MARKER), true)?);
        checkpoint("marker_created")?;
        let marker = stage.marker.as_mut().ok_or_else(invalid)?;
        marker.write_all(MAGIC.as_bytes())?;
        for name in directories {
            writeln!(marker, "D:{name}")?;
        }
        for name in files {
            writeln!(marker, "F:{name}")?;
        }
        checkpoint("marker_written")?;
        marker.sync_all()?;
        checkpoint("marker_synced")?;
        crate::transfer::prepare_destination_folder(&stage.root, &stage.directories)?;
        checkpoint("folders_prepared")?;
        pending.0.take().ok_or_else(invalid)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn display_name(&self) -> Option<String> {
        self.directories
            .first()
            .or_else(|| self.files.first())
            .and_then(|path| path.components().next())
            .and_then(|component| component.as_os_str().to_str().map(str::to_owned))
    }
    pub fn partial_paths(&self) -> Vec<PathBuf> {
        self.partials
            .iter()
            .map(|path| self.root.join(path))
            .collect()
    }
    /// These paths are derived only from admitted transfer IDs, never guessed
    /// by scanning for partial filenames during recovery.
    pub fn register_partials(&mut self, paths: &[String]) -> io::Result<()> {
        if paths.len() > 1024 {
            return Err(invalid());
        }
        let paths = paths
            .iter()
            .map(|path| {
                let relative = relative(path)?;
                let id = relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_prefix(".capstan-"))
                    .and_then(|name| name.strip_suffix(".partial"))
                    .ok_or_else(invalid)?;
                if id.len() != 36
                    || !id
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
                {
                    return Err(invalid());
                }
                Ok(relative)
            })
            .collect::<io::Result<Vec<_>>>()?;
        if !self.partials.is_empty() && self.partials != paths {
            return Err(invalid());
        }
        self.partials = paths;
        self.write_marker()
    }
    fn write_marker(&mut self) -> io::Result<()> {
        let mut body = MAGIC.to_owned();
        for (prefix, paths) in [
            ("D", &self.directories),
            ("F", &self.files),
            ("P", &self.partials),
        ] {
            for path in paths {
                body.push_str(&format!(
                    "{prefix}:{}\n",
                    path.to_string_lossy().replace('\\', "/")
                ));
            }
        }
        if body.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let marker = self.marker.as_mut().ok_or_else(invalid)?;
        marker.seek(SeekFrom::Start(0))?;
        marker.set_len(0)?;
        marker.write_all(body.as_bytes())?;
        marker.sync_all()
    }
    /// Commit the reviewed basename mapping before any download starts.
    pub fn configure(&mut self, directories: &[String], files: &[String]) -> io::Result<()> {
        validate(directories, files)?;
        let directories = directories
            .iter()
            .map(|name| relative(name))
            .collect::<io::Result<Vec<_>>>()?;
        let files = files
            .iter()
            .map(|name| relative(name))
            .collect::<io::Result<Vec<_>>>()?;
        if self.directories == directories && self.files == files {
            return Ok(());
        }
        if !self.directories.is_empty() || !self.files.is_empty() {
            return Err(invalid());
        }
        self.validate_contents()?;
        self.directories = directories;
        self.files = files;
        self.write_marker()
    }
    /// Hold every completed file and its ancestors while Explorer may read it.
    pub fn pin(&mut self) -> io::Result<()> {
        self.validate_contents()?;
        if self
            .partials
            .iter()
            .any(|path| self.root.join(path).try_exists().unwrap_or(true))
        {
            return Err(invalid());
        }
        if self.directory_pins.is_empty() {
            self.directory_pins = self
                .directories
                .iter()
                .map(|path| AnchoredPath::new(&self.root.join(path).join(".capstan-anchor")))
                .collect::<io::Result<_>>()?;
        }
        if self.pins.is_empty() {
            self.pins = self
                .files
                .iter()
                .map(|path| Source::open(&self.root.join(path)))
                .collect::<io::Result<_>>()?;
        }
        for file in &self.pins {
            file.verify()?;
        }
        Ok(())
    }
    pub fn drag_paths(&self) -> io::Result<Vec<PathBuf>> {
        let mut roots = HashSet::new();
        for path in self.directories.iter().chain(&self.files) {
            roots.insert(
                self.root
                    .join(path.components().next().ok_or_else(invalid)?.as_os_str()),
            );
        }
        let mut roots: Vec<_> = roots.into_iter().collect();
        roots.sort();
        Ok(roots)
    }
    /// No recursive deletion. Reject unknown entries/reparse points, then use
    /// exclusive handles so a consumer still reading prevents cleanup.
    pub fn cleanup(&mut self) -> io::Result<()> {
        for file in &self.pins {
            file.verify()?;
        }
        self.validate_contents()?;
        self.pins.clear();
        let mut handles = Vec::new();
        for path in self.files.iter().chain(&self.partials) {
            let anchor = match AnchoredPath::new(&self.root.join(path)) {
                Ok(anchor) => anchor,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            match open_marker(&self.root.join(path), false) {
                Ok(file) => {
                    version(&file, false)?;
                    handles.push((anchor, file));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        // All files must be exclusively open before any deletion is admitted.
        for (_, file) in &handles {
            delete(file)?;
        }
        drop(handles);
        self.directory_pins.clear();
        let mut directories = self.directories.clone();
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            match fs::remove_dir(self.root.join(directory)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(marker) = &self.marker {
            delete(marker)?;
        }
        self.marker.take();
        self.anchor.take();
        remove_directory(&self.root, self.root_identity)
    }
    fn validate_contents(&self) -> io::Result<()> {
        let expected: HashSet<_> = self
            .files
            .iter()
            .chain(&self.directories)
            .chain(&self.partials)
            .cloned()
            .chain([PathBuf::from(MARKER)])
            .collect();
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                let relative = path.strip_prefix(&self.root).map_err(|_| invalid())?;
                if !expected.contains(relative) {
                    return Err(invalid());
                }
                let metadata = fs::symlink_metadata(&path)?;
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                }
                if metadata.is_dir() {
                    pending.push(path);
                }
            }
        }
        Ok(())
    }
    /// Only this app's bounded, unlocked, valid markers are eligible. Unknown
    /// or live-instance directories remain untouched.
    pub fn discover(base: &Path) -> io::Result<Vec<(String, Self)>> {
        let entries = match fs::read_dir(base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut stages = Vec::new();
        for entry in entries.take(256) {
            let entry = entry?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if id.len() != 36 || !id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-') {
                continue;
            }
            let root = entry.path();
            let loaded = (|| {
                let anchor = AnchoredPath::new(&root.join(MARKER))?;
                let root_identity = directory_identity(&root)?;
                let parent = AnchoredPath::new(&root)?;
                let marker = open_marker(&root.join(MARKER), false)?;
                version(&marker, false)?;
                let mut body = String::new();
                (&marker).take(1024 * 1024 + 1).read_to_string(&mut body)?;
                if body.len() > 1024 * 1024 || !body.starts_with(MAGIC) {
                    return Err(invalid());
                }
                let mut directories = Vec::new();
                let mut files = Vec::new();
                let mut partials = Vec::new();
                for line in body[MAGIC.len()..].lines() {
                    if let Some(name) = line.strip_prefix("D:") {
                        directories.push(name.to_owned());
                    } else if let Some(name) = line.strip_prefix("F:") {
                        files.push(name.to_owned());
                    } else if let Some(name) = line.strip_prefix("P:") {
                        let path = relative(name)?;
                        let id = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .and_then(|name| name.strip_prefix(".capstan-"))
                            .and_then(|name| name.strip_suffix(".partial"))
                            .ok_or_else(invalid)?;
                        if id.len() != 36
                            || !id
                                .bytes()
                                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
                            || partials.len() >= 1024
                        {
                            return Err(invalid());
                        }
                        partials.push(path);
                    } else {
                        return Err(invalid());
                    }
                }
                validate(&directories, &files)?;
                Ok(Self {
                    root,
                    root_identity,
                    directories: directories
                        .iter()
                        .map(|name| relative(name))
                        .collect::<io::Result<_>>()?,
                    files: files
                        .iter()
                        .map(|name| relative(name))
                        .collect::<io::Result<_>>()?,
                    marker: Some(marker),
                    anchor: Some(anchor),
                    _parent: parent,
                    pins: Vec::new(),
                    directory_pins: Vec::new(),
                    partials,
                })
            })();
            if let Ok(stage) = loaded {
                stages.push((id, stage));
            }
        }
        Ok(stages)
    }
}
fn directory_identity(path: &Path) -> io::Result<u128> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    Ok(version(&file, true)?.identity)
}
fn remove_directory(path: &Path, identity: u128) -> io::Result<()> {
    // Opening DELETE while the traversal anchor is live would conflict with
    // its sharing policy. After releasing it, validate the original identity
    // on an exclusive directory handle and delete through that same handle.
    let file = OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if version(&file, true)?.identity != identity {
        return Err(invalid());
    }
    delete(&file)
}
fn open_marker(path: &Path, create: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .share_mode(0)
        .create_new(create)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if let Err(error) = validate_marker(&file) {
        // create_new plus the exclusive handle proves ownership even if a
        // subsequent validation call fails; never unlink a discovered file.
        if create {
            let _ = delete(&file);
        }
        return Err(error);
    }
    Ok(file)
}
fn validate_marker(file: &File) -> io::Result<()> {
    version(file, false)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the file and writable structure remain live through the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.nNumberOfLinks != 1 {
        return Err(invalid());
    }
    Ok(())
}
fn delete(file: &File) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the exclusive live handle and correctly sized structure remain live.
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

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "11111111-1111-4111-8111-111111111111";
    #[test]
    fn constructor_failures_roll_back_only_owned_bounded_contents() {
        for failure in [
            "root_identity",
            "restricted",
            "marker_created",
            "marker_written",
            "marker_synced",
            "folders_prepared",
        ] {
            let root = tempfile::tempdir().unwrap();
            let base = root.path().join("exports");
            let unrelated = root.path().join("unrelated");
            fs::write(&unrelated, b"keep").unwrap();
            assert!(ExportStage::prepare_with_checkpoint(
                &base,
                ID,
                &["folder".into(), "folder/nested".into()],
                &[],
                |phase| if phase == failure {
                    Err(io::Error::other("injected initialization failure"))
                } else {
                    Ok(())
                },
            )
            .is_err());
            assert_eq!(fs::read_dir(&base).unwrap().count(), 0, "{failure}");
            assert!(ExportStage::discover(&base).unwrap().is_empty());
            assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
        }
    }
    #[test]
    fn constructor_rollback_preserves_unexpected_contents() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join(ID);
        assert!(
            ExportStage::prepare_with_checkpoint(root.path(), ID, &[], &[], |phase| {
                if phase == "root_identity" {
                    fs::write(stage.join("unknown"), b"keep")?;
                    return Err(io::Error::other("injected initialization failure"));
                }
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read(stage.join("unknown")).unwrap(), b"keep");
        assert!(stage.is_dir());
    }
    #[test]
    fn stage_discovery_ignores_live_owners_and_cleanup_waits_for_readers() {
        let root = tempfile::tempdir().unwrap();
        let mut stage = ExportStage::prepare(
            root.path(),
            ID,
            &["tree".into(), "tree/empty".into()],
            &["tree/file".into()],
        )
        .unwrap();
        fs::write(stage.root().join("tree/file"), b"exported bytes").unwrap();
        stage.pin().unwrap();
        assert!(ExportStage::discover(root.path()).unwrap().is_empty());
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(stage.root().join("tree/file"))
            .unwrap();
        assert!(stage.cleanup().is_err());
        assert_eq!(
            fs::read(stage.root().join("tree/file")).unwrap(),
            b"exported bytes"
        );
        drop(reader);
        drop(stage);
        let mut discovered = ExportStage::discover(root.path()).unwrap();
        assert_eq!(discovered.len(), 1);
        discovered[0].1.cleanup().unwrap();
        assert!(!root.path().join(ID).exists());
    }
    #[test]
    fn unknown_content_and_unsafe_names_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        for bad in ["../escape", "CON", "case/../file", "name.", "file:stream"] {
            assert!(ExportStage::prepare(root.path(), ID, &[], &[bad.into()]).is_err());
        }
        assert!(
            ExportStage::prepare(root.path(), ID, &[], &["case".into(), "CASE".into()]).is_err()
        );
        let mut stage = ExportStage::prepare(root.path(), ID, &[], &["file".into()]).unwrap();
        fs::write(stage.root().join("file"), b"data").unwrap();
        fs::write(stage.root().join("unknown"), b"keep").unwrap();
        assert!(stage.pin().is_err());
        assert!(stage.cleanup().is_err());
        assert_eq!(fs::read(stage.root().join("unknown")).unwrap(), b"keep");
        fs::remove_file(stage.root().join("unknown")).unwrap();
        stage.cleanup().unwrap();
    }
    #[test]
    fn restart_cleanup_consumes_only_registered_partial_names_and_rejects_marker_hard_links() {
        let root = tempfile::tempdir().unwrap();
        let mut stage = ExportStage::prepare(root.path(), ID, &[], &[]).unwrap();
        stage
            .configure(&["folder".into()], &["folder/file".into()])
            .unwrap();
        let name = format!("folder/.capstan-{ID}.partial");
        stage
            .register_partials(std::slice::from_ref(&name))
            .unwrap();
        fs::create_dir(stage.root().join("folder")).unwrap();
        fs::write(stage.root().join(&name), b"partial bytes").unwrap();
        assert!(stage.pin().is_err());
        drop(stage);
        let mut stages = ExportStage::discover(root.path()).unwrap();
        assert_eq!(stages.len(), 1);
        assert_eq!(
            stages[0].1.partial_paths(),
            vec![root.path().join(ID).join(name)]
        );
        stages[0].1.cleanup().unwrap();
        let stage = ExportStage::prepare(root.path(), ID, &[], &[]).unwrap();
        let marker = stage.root().join(MARKER);
        drop(stage);
        let alias = root.path().join("external-marker");
        fs::hard_link(&marker, &alias).unwrap();
        assert!(ExportStage::discover(root.path()).unwrap().is_empty());
        assert_eq!(fs::read(&alias).unwrap(), MAGIC.as_bytes());
    }
    #[test]
    fn final_stage_removal_rejects_a_replaced_directory_identity() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("exports");
        let mut stage = ExportStage::prepare(&base, ID, &[], &[]).unwrap();
        let identity = stage.root_identity;
        let path = stage.root().to_owned();
        stage.marker.take();
        stage.anchor.take();
        let moved = root.path().join("moved");
        fs::rename(&path, &moved).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(remove_directory(&path, identity).is_err());
        assert!(path.is_dir());
        assert!(moved.join(MARKER).exists());
        drop(stage);
    }
}
