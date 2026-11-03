use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault is locked")]
    Locked,
    #[error("vault already exists")]
    AlreadyExists,
    #[error("vault does not exist")]
    NotFound,
    #[error("vault format is invalid")]
    InvalidFormat,
    #[error("vault version is unsupported")]
    UnsupportedVersion,
    #[error("vault KDF parameters exceed policy")]
    KdfPolicy,
    #[error("vault authentication failed")]
    Authentication,
    #[error("vault data is invalid")]
    InvalidData,
    #[error("too many vault recovery candidates")]
    RecoveryLimit,
    #[error("vault revision has changed")]
    Conflict,
    #[error("vault has active references")]
    Referenced,
    #[error("vault filesystem operation failed")]
    Io,
    #[error("vault operation was canceled")]
    Canceled,
    #[error("secure random generation failed")]
    Random,
    #[error("vault import contains trust conflicts")]
    TrustConflict,
}

impl VaultError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Locked => "vault_locked",
            Self::AlreadyExists => "vault_exists",
            Self::NotFound => "vault_not_found",
            Self::InvalidFormat => "vault_format",
            Self::UnsupportedVersion => "vault_version",
            Self::KdfPolicy => "vault_kdf_policy",
            Self::Authentication => "vault_authentication",
            Self::InvalidData => "vault_data",
            Self::RecoveryLimit => "vault_recovery_limit",
            Self::Conflict => "vault_conflict",
            Self::Referenced => "vault_referenced",
            Self::Io => "vault_io",
            Self::Canceled => "canceled",
            Self::Random => "secure_random",
            Self::TrustConflict => "trust_conflict",
        }
    }
}

impl From<std::io::Error> for VaultError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
pub type Result<T> = std::result::Result<T, VaultError>;
