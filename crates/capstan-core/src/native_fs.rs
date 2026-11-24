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
