pub mod config;
pub mod errors;
pub mod gecko;
pub mod parser;
pub mod state;
pub mod types;

use crate::{
    config::{JsonFileConfig, ReplayConfigSource},
    errors::EngineError,
    gecko::GeckoManager,
    parser::{ParsedReplay, ReplayParser, SimpleReplayParser},
    state::PlaybackState,
    types::{FrameDecision, FramePackage, IsReplayReadyResult},
};
use std::path::PathBuf;

use dolphin_integrations::Log;

/// The central orchestrator that wires together the modules.
///
/// HINT: Treat this like your façade. The outside world calls these methods; the
/// internals (config reader, parser, gecko manager) can be swapped via the
/// builder below for tests or alternative implementations.
#[derive(Debug)]
pub struct PlaybackEngine {
    cfg_source: Box<dyn ReplayConfigSource + Send + Sync>,
    parser: Box<dyn ReplayParser + Send + Sync>,
    gecko: GeckoManager,
    state: PlaybackState,
}

impl PlaybackEngine {
    /// Quick-start with JSON config + simple parser.
    pub fn new_with_defaults(config_path: impl Into<PathBuf>) -> Self {
        let config_path = config_path.into();
        tracing::warn!(target: Log::SlippiOnline, "Starting PlaybackEngine with config at {}", config_path.display());

        Self::builder()
            .with_config(JsonFileConfig::new(config_path))
            .with_parser(SimpleReplayParser::default())
            .build()
    }

    /// Builder for injection / customization.
    pub fn builder() -> PlaybackEngineBuilder {
        PlaybackEngineBuilder::default()
    }

    // ─────────────────────────────────────────────────────────────────────
    // 1) is_replay_ready
    // ─────────────────────────────────────────────────────────────────────
    /// Reads current desired replay from config JSON. If it differs from the
    /// currently loaded one, parses it and resets playback state.
    pub fn is_replay_ready(&mut self) -> Result<IsReplayReadyResult, EngineError> {
        tracing::warn!(target: Log::SlippiOnline, "is_replay_ready");

        let cfg = self.cfg_source.read_current()?;
        let desired = cfg.replay_path.clone();

        // TODO: Would it be sufficient for this function to just return a bool?

        // No replay requested
        let Some(desired_path) = desired else {
            return Ok(IsReplayReadyResult::NotReady {
                reason: "no replay path specified".into(),
            });
        };

        // If the desired replay differs from what's currently loaded, (re)load.
        if self.state.current_replay_path.as_ref() != Some(&desired_path) {
            let parsed = self.parser.parse(&desired_path)?;
            self.load_new_game(desired_path, parsed);
            return Ok(IsReplayReadyResult::NewGameLoaded);
        }

        // If already loaded, we consider it ready if minimal data exists.
        if self.state.has_minimum_start_data() {
            Ok(IsReplayReadyResult::Ready)
        } else {
            Ok(IsReplayReadyResult::NotReady {
                reason: "waiting on minimum start data".into(),
            })
        }
    }

    fn load_new_game(&mut self, path: PathBuf, parsed: ParsedReplay) {
        // Reset any playback statuses to prepare for this new replay.
        self.state.reset_for_new_game(path, &parsed);
        // Prepare conditions immediately so `prepare_replay` can be quick.
        self.state.store_start_conditions(parsed.start_conditions);
        self.state.store_frames(parsed.frames);
        self.state.store_initial_rng(parsed.initial_rng_seed);
    }

    // ─────────────────────────────────────────────────────────────────────
    // 2) prepare_replay
    // ─────────────────────────────────────────────────────────────────────
    /// Ensures the replay has enough data to start, fetches start conditions,
    /// and builds the Gecko code list for this match.
    pub fn prepare_replay(&mut self) -> Result<(), EngineError> {
        if !self.state.has_minimum_start_data() {
            return Err(EngineError::InsufficientData("not enough data to start replay".into()));
        }

        // Fetch conditions for match start
        let conditions = self
            .state
            .start_conditions()
            .ok_or_else(|| EngineError::InsufficientData("missing start conditions".into()))?;

        // Build Gecko code list and cache size
        let (codes, total_size) = self.gecko.prepare_for_match(conditions);
        self.state.store_gecko_codes(codes, total_size);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // 3) get_replay_gecko_codelist
    // ─────────────────────────────────────────────────────────────────────
    /// Returns the Gecko code list prepared in `prepare_replay`.
    pub fn get_replay_gecko_codelist(&self) -> Result<(&[u8], usize), EngineError> {
        self.state.gecko_blob().ok_or_else(|| EngineError::GeckoNotPrepared)
    }

    // ─────────────────────────────────────────────────────────────────────
    // 4) prepare_replay_frame
    // ─────────────────────────────────────────────────────────────────────
    /// Decides whether to play/halt/ff/terminate and returns the inputs + rng
    /// for the current frame (if any).
    pub fn prepare_replay_frame(&mut self) -> Result<(FrameDecision, Option<FramePackage>), EngineError> {
        // Basic decision logic stub — customize as needed.
        let decision = self.state.compute_frame_decision();

        match decision {
            FrameDecision::Terminate => Ok((FrameDecision::Terminate, None)),
            FrameDecision::Halt => Ok((FrameDecision::Halt, None)),
            FrameDecision::FastForward | FrameDecision::Play => {
                let maybe_pkg = self.state.next_frame_package()?;
                Ok((decision, maybe_pkg))
            },
        }
    }
}

#[derive(Default)]
pub struct PlaybackEngineBuilder {
    cfg_source: Option<Box<dyn ReplayConfigSource + Send + Sync>>,
    parser: Option<Box<dyn ReplayParser + Send + Sync>>,
}

impl PlaybackEngineBuilder {
    pub fn with_config(mut self, src: impl ReplayConfigSource + Send + Sync + 'static) -> Self {
        self.cfg_source = Some(Box::new(src));
        self
    }
    pub fn with_parser(mut self, parser: impl ReplayParser + Send + Sync + 'static) -> Self {
        self.parser = Some(Box::new(parser));
        self
    }
    pub fn build(self) -> PlaybackEngine {
        PlaybackEngine {
            cfg_source: self
                .cfg_source
                .unwrap_or_else(|| Box::new(JsonFileConfig::new("replay_config.json"))),
            parser: self.parser.unwrap_or_else(|| Box::new(SimpleReplayParser::default())),
            gecko: GeckoManager::default(),
            state: PlaybackState::default(),
        }
    }
}
