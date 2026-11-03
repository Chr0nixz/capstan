use super::WinChild;
use crate::cmdbuilder::CommandBuilder;
use crate::win::procthreadattr::ProcThreadAttributeList;
use anyhow::{bail, ensure, Error};
use filedescriptor::{FileDescriptor, OwnedHandle};
use shared_library::shared_library;
use std::io::Error as IoError;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::sync::Mutex;
use std::{mem, ptr};
use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::{HRESULT, S_OK};
use winapi::um::handleapi::*;
use winapi::um::processthreadsapi::*;
use winapi::um::winbase::{
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use winapi::um::wincon::COORD;
use winapi::um::winnt::HANDLE;

pub type HPCON = HANDLE;

pub const PSEUDOCONSOLE_RESIZE_QUIRK: DWORD = 0x2;
pub const PSEUDOCONSOLE_WIN32_INPUT_MODE: DWORD = 0x4;
#[allow(dead_code)]
pub const PSEUDOCONSOLE_PASSTHROUGH_MODE: DWORD = 0x8;

shared_library!(ConPtyFuncs,
    pub fn CreatePseudoConsole(
        size: COORD,
        hInput: HANDLE,
        hOutput: HANDLE,
        flags: DWORD,
        hpc: *mut HPCON
    ) -> HRESULT,
    pub fn ResizePseudoConsole(hpc: HPCON, size: COORD) -> HRESULT,
    pub fn ClosePseudoConsole(hpc: HPCON),
);

shared_library!(BundledConPtyFuncs,
    pub fn ConptyCreatePseudoConsole(
        size: COORD, hInput: HANDLE, hOutput: HANDLE, flags: DWORD, hpc: *mut HPCON
    ) -> HRESULT,
    pub fn ConptyResizePseudoConsole(hpc: HPCON, size: COORD) -> HRESULT,
    pub fn ConptyClosePseudoConsole(hpc: HPCON),
);

// Each table owns its module for at least as long as the corresponding HPCON.
// Bundled NuGet runtimes use Conpty-prefixed exports; older side-loaded DLLs
// expose the kernel32-compatible aliases.
enum RuntimeFunctions {
    System(ConPtyFuncs),
    Bundled(BundledConPtyFuncs),
}
impl RuntimeFunctions {
    unsafe fn create(
        &self,
        size: COORD,
        input: HANDLE,
        output: HANDLE,
        flags: DWORD,
        con: *mut HPCON,
    ) -> HRESULT {
        match self {
            Self::System(api) => (api.CreatePseudoConsole)(size, input, output, flags, con),
            Self::Bundled(api) => (api.ConptyCreatePseudoConsole)(size, input, output, flags, con),
        }
    }
    unsafe fn resize(&self, con: HPCON, size: COORD) -> HRESULT {
        match self {
            Self::System(api) => (api.ResizePseudoConsole)(con, size),
            Self::Bundled(api) => (api.ConptyResizePseudoConsole)(con, size),
        }
    }
    unsafe fn close(&self, con: HPCON) {
        match self {
            Self::System(api) => (api.ClosePseudoConsole)(con),
            Self::Bundled(api) => (api.ConptyClosePseudoConsole)(con),
        }
    }
}

// Never resolve an unverified conpty.dll through the process DLL search path.
fn load_conpty(path: Option<&Path>) -> Result<RuntimeFunctions, Error> {
    if let Some(path) = path {
        ensure!(path.is_absolute(), "ConPTY path must be absolute");
        if let Ok(api) = BundledConPtyFuncs::open(path) {
            return Ok(RuntimeFunctions::Bundled(api));
        }
        ConPtyFuncs::open(path)
            .map(RuntimeFunctions::System)
            .map_err(|_| anyhow::anyhow!("failed to load configured ConPTY runtime"))
    } else {
        // kernel32 is a Windows KnownDLL, not an application search-path DLL.
        ConPtyFuncs::open(Path::new("kernel32.dll"))
            .map(RuntimeFunctions::System)
            .map_err(|_| anyhow::anyhow!("system ConPTY is unavailable"))
    }
}

pub struct PsuedoCon {
    con: HPCON,
    functions: RuntimeFunctions,
}

unsafe impl Send for PsuedoCon {}
unsafe impl Sync for PsuedoCon {}

impl Drop for PsuedoCon {
    fn drop(&mut self) {
        unsafe { self.functions.close(self.con) };
    }
}

impl PsuedoCon {
    pub fn new(
        size: COORD,
        input: FileDescriptor,
        output: FileDescriptor,
        runtime: Option<&Path>,
    ) -> Result<Self, Error> {
        let functions = load_conpty(runtime)?;
        let mut con: HPCON = INVALID_HANDLE_VALUE;
        let result = unsafe {
            functions.create(
                size,
                input.as_raw_handle() as _,
                output.as_raw_handle() as _,
                // Each Capstan pane starts with its own cursor. Inheriting a
                // parent console cursor requires a reply during initialization
                // and can deadlock startup/close before a frontend is attached.
                PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE,
                &mut con,
            )
        };
        ensure!(
            result == S_OK,
            "failed to create psuedo console: HRESULT {}",
            result
        );
        Ok(Self { con, functions })
    }

    pub fn resize(&self, size: COORD) -> Result<(), Error> {
        let result = unsafe { self.functions.resize(self.con, size) };
        ensure!(
            result == S_OK,
            "failed to resize console to {}x{}: HRESULT: {}",
            size.X,
            size.Y,
            result
        );
        Ok(())
    }

    pub fn spawn_command(&self, cmd: CommandBuilder) -> anyhow::Result<WinChild> {
        let mut si: STARTUPINFOEXW = unsafe { mem::zeroed() };
        si.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        // Explicitly set the stdio handles as invalid handles otherwise
        // we can end up with a weird state where the spawned process can
        // inherit the explicitly redirected output handles from its parent.
        // For example, when daemonizing wezterm-mux-server, the stdio handles
        // are redirected to a log file and the spawned process would end up
        // writing its output there instead of to the pty we just created.
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;

        let mut attrs = ProcThreadAttributeList::with_capacity(1)?;
        attrs.set_pty(self.con)?;
        si.lpAttributeList = attrs.as_mut_ptr();

        let mut pi: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        let (mut exe, mut cmdline) = cmd.cmdline()?;
        let cwd = cmd.current_directory();

        let res = unsafe {
            CreateProcessW(
                exe.as_mut_slice().as_mut_ptr(),
                cmdline.as_mut_slice().as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                cmd.environment_block().as_mut_slice().as_mut_ptr() as *mut _,
                cwd.as_ref()
                    .map(|c| c.as_slice().as_ptr())
                    .unwrap_or(ptr::null()),
                &mut si.StartupInfo,
                &mut pi,
            )
        };
        if res == 0 {
            let err = IoError::last_os_error();
            // Arguments and environment values may contain credentials.
            bail!("CreateProcessW failed: {}", err);
        }

        // Make sure we close out the thread handle so we don't leak it;
        // we do this simply by making it owned
        let _main_thread = unsafe { OwnedHandle::from_raw_handle(pi.hThread as _) };
        let proc = unsafe { OwnedHandle::from_raw_handle(pi.hProcess as _) };

        Ok(WinChild {
            proc: Mutex::new(proc),
        })
    }
}
