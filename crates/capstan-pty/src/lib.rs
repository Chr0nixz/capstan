//! Platform-neutral building blocks for Capstan's local terminal path.
//!
//! The crate deliberately has no Tauri dependency.  The application layer can
//! adapt [`BinaryFrame`] to a Tauri Channel while the shell discovery,
//! backpressure and resource lifecycle remain independently testable.

#![deny(unsafe_code)]
#![allow(clippy::pedantic)]

#[cfg(feature = "m0-benchmarks")]
pub mod benchmark;
mod conpty;
mod error;
mod local;
mod session;
mod shell;
mod wsl;
pub use local::{
    local_shell_profiles, open_local_terminal, LocalShellProfile, LocalTerminalOptions,
};

pub use capstan_terminal::{
    AckResult, BackpressureGate, BinaryFrame, FrameSink, OutputConfig, OutputMerger, OutputMetrics,
    OutputPump, OutputPumpHandle, OutputStream, ACK_RESUME_THRESHOLD, ACK_SUSPEND_THRESHOLD,
    FLUSH_INTERVAL, MAX_FRAME_SIZE,
};
pub use conpty::{
    select_conpty, validate_bundle_path, ConPtyConfig, ConPtySelection, ConPtySource, ConPtyStatus,
};
pub use error::{PtyError, Result};
pub use session::{PtyCommand, PtySession, PtySessionConfig, SessionExit, SessionState};
pub use shell::{PowerShellPreference, ShellDetector, ShellInfo, ShellKind, ShellSource};
pub use wsl::{
    parse_wsl_verbose_output, WslDistribution, WslProbe, WslProbeConfig, WslProbeResult, WslSource,
};
