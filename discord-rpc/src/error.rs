use thiserror::Error;

use crate::Message;

/// Any error type that can be raised by this library.
#[derive(Error, Debug)]
pub enum DiscordRPCError {
    #[error("{0}")]
    GenericIO(#[from] std::io::Error),

    #[error("Failed to spawn thread: {0}")]
    ThreadSpawn(std::io::Error),

    #[error("The channel receiver has disconnected, implying that the data could never be received.")]
    ChannelReceiverDisconnected(#[from] std::sync::mpsc::SendError<Message>),

    #[error("The channel sender has disconnected, implying no further messages will be received.")]
    ChannelSenderDisconnected(#[from] std::sync::mpsc::RecvError),

    #[error("Unknown DiscordRPC Error")]
    Unknown,
}
