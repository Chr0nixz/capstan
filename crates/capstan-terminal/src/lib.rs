//! Backend-neutral terminal transport and resource ownership.
#![deny(unsafe_code)]
#![allow(clippy::pedantic)]
mod error;
mod output;
mod service;
pub use error::{Result, TerminalError};
pub use output::*;
pub use service::*;

mod protocol;
pub use protocol::encode_binary_frame;
