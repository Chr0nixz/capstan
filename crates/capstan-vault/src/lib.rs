//! Authenticated vault envelopes, domain validation and crash-safe persistence.
#![deny(unsafe_code)]
#![allow(clippy::pedantic)]
mod envelope;
mod error;
mod model;
mod store;
pub use envelope::{EnvelopeKind, KdfParameters, MAX_FILE_BYTES};
pub use error::{Result, VaultError};
pub use model::*;
pub use store::*;
