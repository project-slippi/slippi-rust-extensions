use thiserror::Error;

#[derive(Error, Debug)]
pub enum DiscordRpcError {
    #[error("Failed to spawn thread: {0}")]
    ThreadSpawn(std::io::Error),

    #[error("The channel sender has disconnected, implying no further messages will be received.")]
    ChannelDisconnected(#[from] std::sync::mpsc::RecvTimeoutError),
}
