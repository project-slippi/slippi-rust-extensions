use crate::errors::EngineError;
use crate::types::{FrameInputs, StartConditions};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ParsedReplay {
    pub frames: Vec<FrameInputs>,
    pub start_conditions: StartConditions,
    pub initial_rng_seed: u32,
}

pub trait ReplayParser: std::fmt::Debug {
    fn parse(&self, path: &Path) -> Result<ParsedReplay, EngineError>;
}

/// Minimal placeholder parser. Replace with your actual replay format parser.
#[derive(Default, Debug)]
pub struct SimpleReplayParser;

impl ReplayParser for SimpleReplayParser {
    fn parse(&self, path: &Path) -> Result<ParsedReplay, EngineError> {
        // HINT: Replace this with real parsing code.
        // Here we just require the file to exist and return fake data.
        let _ = fs::read(path).map_err(|e| EngineError::ReplayIo(format!("{}: {e}", path.display())))?;

        let frames = vec![FrameInputs::default(); 10];
        let start_conditions = StartConditions {
            stage_id: 2,
            characters: vec![1, 2],
            settings_blob: vec![],
        };
        let initial_rng_seed = 0x1234_5678;

        Ok(ParsedReplay {
            frames,
            start_conditions,
            initial_rng_seed,
        })
    }
}
