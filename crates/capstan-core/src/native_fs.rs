//! Platform local-file capabilities for picker-selected, dropped or
//! application-owned paths. Both platforms expose the same API: Windows uses
//! the handle-pinning implementation in `capstan-native-windows`, Unix uses
//! [`unix`]. Callers never receive these paths from the WebView.
use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(windows)]
pub use capstan_native_windows::transfer::*;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

/// What a selected folder upload sends. Links, junctions and special files
/// are never followed or copied; they are only counted in `skipped`.
pub struct FolderContents {
    pub directories: Vec<PathBuf>,
    pub files: Vec<(PathBuf, Source)>,
    pub skipped: u32,
}

/// Whether a selected or dropped entry is a link, junction, other reparse
/// point or special file. Uploads leave these out, as inside folders, instead
/// of following them. `metadata` must come from `symlink_metadata`.
pub(crate) fn is_skipped_entry(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    !(metadata.is_dir() || metadata.is_file())
}

#[cfg(windows)]
pub fn scan_folder(path: &Path) -> io::Result<FolderContents> {
    let directories = enumerate_folder_directories(path)?;
    let (files, skipped) = enumerate_folder(path)?;
    Ok(FolderContents {
        directories,
        files,
        skipped,
    })
}

pub(crate) fn prepare_selected_folder(
    directory: &SelectedDirectory,
    paths: &[PathBuf],
) -> io::Result<()> {
    #[cfg(windows)]
    return prepare_destination_folder(directory.root(), paths);
    #[cfg(unix)]
    directory.prepare(paths)
}

pub(crate) fn select_folder_destination(
    directory: &SelectedDirectory,
    relative: &Path,
) -> io::Result<Destination> {
    #[cfg(windows)]
    return Destination::select_under_folder(directory.root(), relative);
    #[cfg(unix)]
    directory.select_destination(relative)
}
