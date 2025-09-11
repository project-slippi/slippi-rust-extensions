use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("config io error: {0}")]
    ConfigIo(String),
    #[error("config parse error: {0}")]
    ConfigParse(String),
    #[error("replay io error: {0}")]
    ReplayIo(String),
    #[error("replay parse error: {0}")]
    ReplayParse(String),
    #[error("insufficient data: {0}")]
    InsufficientData(String),
    #[error("gecko list not prepared yet")]
    GeckoNotPrepared,
    #[error("frame index out of range: {0:?}")]
    FrameOutOfRange(PathBuf),
}
