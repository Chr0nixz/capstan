//! Bounded SFTP facade and framing adaptation.
//!
//! This is a framing adapter, not a second SFTP request dispatcher. Request IDs,
//! timeouts and protocol operations remain owned by the selected library.
mod wire;
pub(crate) use wire::SftpReaper;
pub use wire::{SftpWire, MAX_PACKET_BYTES};
mod client;
pub(crate) use client::Admission;
pub use client::{
    Attributes, Capabilities, ClientError, PipelinedFile, RemoteDirectory, RemoteFile, SftpClient,
};
