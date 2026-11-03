//! M2 platform probe: copy-only Shell drag data for already-staged local items.
//!
//! This module does not download, grant paths, own staging cleanup, or expose a
//! WebView command. Its caller must retain the export until consumers finish;
//! DoDragDrop returning does not prove an asynchronous Explorer copy finished.
use std::{
    ffi::c_void,
    io,
    marker::PhantomData,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    ptr,
    rc::Rc,
};
use windows_sys::{
    core::{GUID, HRESULT},
    Win32::{
        Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, HWND},
        System::{
            Com::CoTaskMemFree,
            Ole::{OleInitialize, OleUninitialize, DROPEFFECT_COPY},
        },
        UI::Shell::{
            Common::ITEMIDLIST, ILFindLastID, SHCreateDataObject, SHDoDragDrop, SHParseDisplayName,
        },
    },
};

const IID_DATA_OBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const MAX_DRAG_ITEMS: usize = 256;

#[repr(C)]
struct UnknownVtable {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn initialize() -> io::Result<Self> {
        // SAFETY: null is the required reserved argument. The !Send guard calls
        // OleUninitialize on this exact thread, including S_FALSE success.
        check(unsafe { OleInitialize(ptr::null()) })?;
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this guard owns one successful initialization on this thread.
        unsafe { OleUninitialize() };
    }
}

struct ItemId(*mut ITEMIDLIST);
impl Drop for ItemId {
    fn drop(&mut self) {
        // SAFETY: SHParseDisplayName transfers its allocation to the caller.
        unsafe { CoTaskMemFree(self.0.cast()) };
    }
}

/// Apartment-bound OLE object. Only Rust-selected staged paths may reach here.
pub struct StagedDrag {
    object: *mut c_void,
    _apartment: Apartment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    Copied,
    Canceled,
}

impl StagedDrag {
    pub fn prepare(root: &Path, paths: &[PathBuf]) -> io::Result<Self> {
        if !root.is_absolute() || paths.is_empty() || paths.len() > MAX_DRAG_ITEMS {
            return Err(invalid_input());
        }
        let root_metadata = std::fs::symlink_metadata(root)?;
        if !root_metadata.is_dir() || is_reparse(&root_metadata) {
            return Err(invalid_input());
        }
        let apartment = Apartment::initialize()?;
        let mut root_wide: Vec<u16> = root.as_os_str().encode_wide().collect();
        if root_wide.contains(&0) || root_wide.len() > 32766 {
            return Err(invalid_input());
        }
        root_wide.push(0);
        let mut parent_id = ptr::null_mut();
        // SAFETY: live null-terminated path and out pointer. ItemId owns the
        // transferred allocation until SHCreateDataObject has copied it.
        check(unsafe {
            SHParseDisplayName(
                root_wide.as_ptr(),
                ptr::null_mut(),
                &mut parent_id,
                0,
                ptr::null_mut(),
            )
        })?;
        if parent_id.is_null() {
            return Err(invalid_input());
        }
        let parent_id = ItemId(parent_id);
        let mut identifiers = Vec::with_capacity(paths.len());
        for path in paths {
            let relative = path.strip_prefix(root).map_err(|_| invalid_input())?;
            if relative.as_os_str().is_empty()
                || relative.components().count() != 1
                || !relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
            {
                return Err(invalid_input());
            }
            let mut current = root.to_path_buf();
            for component in relative.components() {
                current.push(component);
                let metadata = std::fs::symlink_metadata(&current)?;
                if is_reparse(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
                    return Err(invalid_input());
                }
            }
            let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
            if wide.contains(&0) || wide.len() > 32766 {
                return Err(invalid_input());
            }
            wide.push(0);
            let mut identifier = ptr::null_mut();
            // SAFETY: null-terminated path and live out pointer; no binding
            // context is needed for these existing filesystem paths.
            check(unsafe {
                SHParseDisplayName(
                    wide.as_ptr(),
                    ptr::null_mut(),
                    &mut identifier,
                    0,
                    ptr::null_mut(),
                )
            })?;
            if identifier.is_null() {
                return Err(invalid_input());
            }
            identifiers.push(ItemId(identifier));
        }
        // SAFETY: each absolute PIDL is retained in identifiers. Its last
        // segment is relative to the common parent parsed above.
        let pointers: Vec<*const ITEMIDLIST> = identifiers
            .iter()
            .map(|item| unsafe { ILFindLastID(item.0).cast_const() })
            .collect();
        let mut object = ptr::null_mut();
        // SAFETY: parent and child PIDLs belong to one folder. Shell copies
        // them into the new IDataObject; all input allocations remain live.
        check(unsafe {
            SHCreateDataObject(
                parent_id.0,
                pointers.len() as u32,
                pointers.as_ptr(),
                ptr::null_mut(),
                &IID_DATA_OBJECT,
                &mut object,
            )
        })?;
        if object.is_null() {
            return Err(invalid_input());
        }
        Ok(Self {
            object,
            _apartment: apartment,
        })
    }

    /// Call from the native UI thread while handling the drag gesture.
    /// The platform default drop source supplies Escape/button cancellation.
    pub fn start(&self, parent: isize) -> io::Result<DragOutcome> {
        let mut effect = 0;
        // SAFETY: apartment and Shell IDataObject live through the blocking OLE
        // loop. Only COPY is allowed; an external target cannot delete sources.
        let result = unsafe {
            SHDoDragDrop(
                parent as HWND,
                self.object,
                ptr::null_mut(),
                DROPEFFECT_COPY,
                &mut effect,
            )
        };
        match result {
            DRAGDROP_S_DROP if effect == DROPEFFECT_COPY => Ok(DragOutcome::Copied),
            DRAGDROP_S_CANCEL => Ok(DragOutcome::Canceled),
            result if result >= 0 && effect == 0 => Ok(DragOutcome::Canceled),
            _ => Err(io::Error::other("native file drag failed")),
        }
    }
}
impl Drop for StagedDrag {
    fn drop(&mut self) {
        // SAFETY: object is a live IDataObject with the standard IUnknown prefix
        // and one reference from SHCreateDataObject. Release precedes apartment.
        unsafe {
            let table = *self.object.cast::<*const UnknownVtable>();
            ((*table).release)(self.object);
        }
    }
}

fn check(result: HRESULT) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::other(format!(
            "native file drag setup failed ({result:#010x})"
        )))
    } else {
        Ok(())
    }
}
fn invalid_input() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid staged export")
}
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL},
            Ole::{ReleaseStgMedium, CF_HDROP},
        },
        UI::Shell::DragQueryFileW,
    };
    #[repr(C)]
    struct DataVtable {
        unknown: UnknownVtable,
        get_data:
            unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    }

    #[test]
    fn shell_data_object_contains_exact_unicode_file_and_directory_paths() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file space 中文.txt");
        let directory = root.path().join("folder 中文");
        std::fs::write(&file, b"probe").unwrap();
        std::fs::create_dir(&directory).unwrap();
        let expected = vec![file, directory];
        let drag = StagedDrag::prepare(root.path(), &expected).unwrap();
        let format = FORMATETC {
            cfFormat: CF_HDROP,
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL as u32,
            ..Default::default()
        };
        let mut medium = STGMEDIUM::default();
        // SAFETY: live IDataObject, correct GetData vtable prefix and writable
        // medium. The HGLOBAL is only inspected until ReleaseStgMedium below.
        unsafe {
            let table = *drag.object.cast::<*const DataVtable>();
            check(((*table).get_data)(drag.object, &format, &mut medium)).unwrap();
            assert_eq!(medium.tymed, TYMED_HGLOBAL as u32);
            let count = DragQueryFileW(medium.u.hGlobal.cast(), u32::MAX, ptr::null_mut(), 0);
            assert_eq!(count as usize, expected.len());
            let mut actual = Vec::new();
            for index in 0..count {
                let length = DragQueryFileW(medium.u.hGlobal.cast(), index, ptr::null_mut(), 0);
                let mut wide = vec![0; length as usize + 1];
                assert_eq!(
                    DragQueryFileW(
                        medium.u.hGlobal.cast(),
                        index,
                        wide.as_mut_ptr(),
                        wide.len() as u32
                    ),
                    length
                );
                actual.push(PathBuf::from(std::ffi::OsString::from_wide(
                    &wide[..length as usize],
                )));
            }
            ReleaseStgMedium(&mut medium);
            assert_eq!(actual, expected);
        }
        assert!(
            root.path().exists(),
            "data object release does not delete staging"
        );
    }

    #[test]
    fn staged_export_rejects_escape_missing_items_and_unbounded_lists() {
        let root = tempfile::tempdir().unwrap();
        for paths in [
            vec![],
            vec![root.path().to_path_buf()],
            vec![root.path().join("../outside")],
            vec![root.path().join("missing")],
            vec![root.path().join("missing"); 257],
        ] {
            assert!(StagedDrag::prepare(root.path(), &paths).is_err());
        }
        assert!(StagedDrag::prepare(Path::new("relative"), &[root.path().join("file")]).is_err());
    }
}
