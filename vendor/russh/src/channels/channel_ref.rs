use tokio::sync::mpsc::Sender;

use std::sync::Arc;

use super::{ChannelProgress, WindowSizeRef};
use crate::ChannelMsg;

/// A handle to the [`super::Channel`]'s to be able to transmit messages
/// to it and update it's `window_size`.
#[derive(Debug)]
pub struct ChannelRef {
    pub(super) sender: Sender<ChannelMsg>,
    pub(super) window_size: WindowSizeRef,
    pub(crate) progress: Option<Arc<ChannelProgress>>,
}

impl ChannelRef {
    pub fn new(sender: Sender<ChannelMsg>) -> Self {
        Self {
            sender,
            window_size: WindowSizeRef::new(0),
            progress: None,
        }
    }

    pub(crate) fn with_progress(
        sender: Sender<ChannelMsg>,
        progress: Arc<ChannelProgress>,
    ) -> Self {
        Self {
            sender,
            window_size: WindowSizeRef::new(0),
            progress: Some(progress),
        }
    }

    pub(crate) fn window_size(&self) -> &WindowSizeRef {
        &self.window_size
    }
}

impl std::ops::Deref for ChannelRef {
    type Target = Sender<ChannelMsg>;

    fn deref(&self) -> &Self::Target {
        &self.sender
    }
}
