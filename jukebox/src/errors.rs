use thiserror::Error;

#[derive(Error, Debug)]
pub enum JukeboxError {
    #[error("{0}")]
    GenericIO(#[from] std::io::Error),

    #[error("The desired offset ({0}) doesn't exist in the compressed ISO")]
    OffsetMissingFromCompressedIso(u64),

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
