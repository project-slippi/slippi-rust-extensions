use crate::errors::EngineError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineConfig {
    /// Absolute or relative path to the replay file to load.
    pub replay_path: Option<PathBuf>,
    // You can add transient toggles here (speed, pause, etc) if desired.
}

pub trait ReplayConfigSource: std::fmt::Debug {
    fn read_current(&self) -> Result<EngineConfig, EngineError>;
}

/// Reads a JSON file off disk every time `read_current` is called.
///
/// HINT: This is intentionally stateless so external tools can flip the desired
/// replay by rewriting the JSON file.
#[derive(Debug, Clone)]
pub struct JsonFileConfig {
    path: PathBuf,
}

impl JsonFileConfig {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ReplayConfigSource for JsonFileConfig {
    fn read_current(&self) -> Result<EngineConfig, EngineError> {
        let txt = fs::read_to_string(&self.path).map_err(|e| EngineError::ConfigIo(format!("{}: {e}", self.path.display())))?;
        serde_json::from_str::<EngineConfig>(&txt).map_err(|e| EngineError::ConfigParse(format!("{}: {e}", self.path.display())))
    }
}
