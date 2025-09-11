/// Outcome of calling `is_replay_ready`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsReplayReadyResult {
    /// No replay path or missing minimum info.
    NotReady { reason: String },
    /// Newly detected replay was parsed and state was reset.
    NewGameLoaded,
    /// The currently loaded replay appears ready to start.
    Ready,
}

/// What to do with the current frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDecision {
    Play,
    Halt,
    FastForward,
    Terminate,
}

/// Stub for per-player controller inputs. Expand as needed.
#[derive(Debug, Clone, Default)]
pub struct ControllerInput {
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    pub l_analog: u8,
    pub r_analog: u8,
    pub stick_x: i8,
    pub stick_y: i8,
    // TODO: Add the real fields from your game.
}

/// Per-frame inputs (for all players) plus any other frame-level metadata.
#[derive(Debug, Clone, Default)]
pub struct FrameInputs {
    pub players: Vec<ControllerInput>,
}

/// What the parser extracts that is needed to start the match.
#[derive(Debug, Clone, Default)]
pub struct StartConditions {
    // TODO: Fill in with your real types
    pub stage_id: u16,
    pub characters: Vec<u16>,
    pub settings_blob: Vec<u8>,
}

/// The full payload needed to play a frame.
#[derive(Debug, Clone)]
pub struct FramePackage {
    pub frame_index: usize,
    pub inputs: FrameInputs,
    pub rng_seed: u32,
}
