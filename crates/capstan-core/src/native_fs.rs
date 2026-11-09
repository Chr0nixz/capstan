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

pub type FolderContents = (Vec<PathBuf>, Vec<(PathBuf, Source)>);

#[cfg(windows)]
pub fn scan_folder(path: &Path) -> io::Result<FolderContents> {
    Ok((enumerate_folder_directories(path)?, enumerate_folder(path)?))
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
