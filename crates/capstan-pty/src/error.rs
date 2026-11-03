use std::io;

use thiserror::Error;

/// Errors exposed by the local terminal adapter.
#[derive(Debug, Error)]
pub enum PtyError {
    #[error("invalid terminal size: columns={columns}, rows={rows}")]
    InvalidSize { columns: u16, rows: u16 },
    #[error("shell executable does not exist: {0}")]
    ShellNotFound(String),
    #[error("no suitable shell was found")]
    NoShell,
    #[error("working directory does not exist or is invalid")]
    InvalidWorkingDirectory,
    #[error("WSL probe failed: {0}")]
    WslProbe(String),
    #[error("ConPTY runtime error: {0}")]
    ConPty(String),
    #[error("PTY error: {0}")]
    Pty(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("output channel closed")]
    OutputClosed,
    #[error("output channel send failed: {0}")]
    OutputSend(String),
    #[error("ACK of {acknowledged} bytes exceeds {in_flight} bytes in flight")]
    AckExceedsInFlight {
        acknowledged: usize,
        in_flight: usize,
    },
    #[error("terminal output was cancelled")]
    Cancelled,
}

impl PtyError {
    /// Stable, content-free identifier for metrics and user-facing error
    /// mapping.  Error strings are intentionally not used as telemetry keys.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSize { .. } => "invalid_size",
            Self::ShellNotFound(_) => "shell_not_found",
            Self::NoShell => "no_shell",
            Self::InvalidWorkingDirectory => "working_directory_invalid",
            Self::WslProbe(_) => "wsl_probe",
            Self::ConPty(_) => "conpty",
            Self::Pty(_) => "pty",
            Self::Io(_) => "io",
            Self::OutputClosed => "output_closed",
            Self::OutputSend(_) => "output_send",
            Self::AckExceedsInFlight { .. } => "ack_exceeds_in_flight",
            Self::Cancelled => "cancelled",
        }
    }
}

pub type Result<T> = std::result::Result<T, PtyError>;

#[cfg(test)]
mod tests {
    use super::PtyError;

    #[test]
    fn error_codes_are_stable_and_content_free() {
        assert_eq!(PtyError::NoShell.code(), "no_shell");
        assert_eq!(
            PtyError::OutputSend("detail".to_owned()).code(),
            "output_send"
        );
    }
}
