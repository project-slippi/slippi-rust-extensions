use thiserror::Error;

use crate::Message;

#[derive(Error, Debug)]
pub enum JukeboxError {
    #[error("{0}")]
    GenericIO(#[from] std::io::Error),

    #[error("Failed to spawn thread: {0}")]
    ThreadSpawn(std::io::Error),

    #[error("The channel receiver has disconnected, implying that the data could never be received.")]
    ChannelReceiverDisconnected(#[from] std::sync::mpsc::SendError<Message>),

    #[error("The channel sender has disconnected, implying no further messages will be received.")]
    ChannelSenderDisconnected(#[from] std::sync::mpsc::RecvError),

    #[error("Failed to decode music file: {0}")]
    MusicFileDecoding(#[from] hps_decode::hps::HpsParseError),

    #[error("Unable to get an audio device handle: {0}")]
    AudioDevice(#[from] rodio::StreamError),

    #[error("Unable to play sound with rodio: {0}")]
    AudioPlayback(#[from] rodio::PlayError),

    #[error("Failed to seek the ISO: {0}")]
    IsoSeek(std::io::Error),

    #[error("Failed to read the ISO: {0}")]
    IsoRead(std::io::Error),

    #[error("The provided game file is not supported")]
    UnsupportedIso,

    #[error("Unknown Jukebox Error")]
    Unknown,
}
