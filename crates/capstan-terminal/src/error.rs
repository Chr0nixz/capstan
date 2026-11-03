use thiserror::Error;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("terminal output is closed")]
    OutputClosed,
    #[error("terminal output send failed: {0}")]
    OutputSend(String),
    #[error("terminal queue is full")]
    QueueFull,
    #[error("terminal input exceeds the maximum chunk size")]
    ChunkTooLarge,
    #[error("ACK of {acknowledged} bytes exceeds {in_flight} bytes in flight")]
    AckExceedsInFlight {
        acknowledged: usize,
        in_flight: usize,
    },
    #[error("terminal operation was cancelled")]
    Cancelled,
    #[error("terminal backend failed: {0}")]
    Backend(String),
    /// A backend's fixed public error identifier, never an error message or
    /// user-supplied text. Dynamic backend details remain redacted by Backend.
    #[error("terminal backend rejected operation: {0}")]
    BackendCode(&'static str),
    #[error("terminal session is closed")]
    SessionClosed,
    #[error("terminal acknowledgement timed out")]
    AckTimeout,
    #[error("terminal task failed")]
    TaskFailed,
    #[error("invalid terminal dimensions")]
    InvalidSize,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl TerminalError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::OutputClosed => "output_closed",
            Self::OutputSend(_) => "output_send",
            Self::QueueFull => "queue_full",
            Self::ChunkTooLarge => "chunk_too_large",
            Self::AckExceedsInFlight { .. } => "ack_exceeds_in_flight",
            Self::Cancelled => "cancelled",
            Self::Backend(_) => "backend",
            Self::BackendCode(code) => code,
            Self::SessionClosed => "session_closed",
            Self::AckTimeout => "ack_timeout",
            Self::TaskFailed => "task_failed",
            Self::InvalidSize => "invalid_size",
            Self::Io(_) => "io",
        }
    }
}

pub type Result<T> = std::result::Result<T, TerminalError>;
