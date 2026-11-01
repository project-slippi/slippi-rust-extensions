//! C FFI for match quality. See the `slippi-match-quality` crate for how matches are judged.
//!
//! These functions are all called from the CPU thread.

use slippi_match_quality::{Frame, GameConfig};

/// Settings for a game, passed when it starts.
#[repr(C)]
pub struct SlippiMatchQualityGameConfig {
    /// Whether the match may be terminated (ranked). Other modes are tracked and logged only.
    pub can_terminate: bool,
    /// The local player's delay frames.
    pub delay: u8,
}

/// One pass through the online frame handler.
#[repr(C)]
pub struct SlippiMatchQualityFrame {
    pub frame: i32,
    pub time_us: u64,
    /// The frame is halted because the opponent's inputs are past the rollback limit.
    pub is_stall: bool,
    /// The newest frame we have the opponent's input for.
    pub latest_remote_frame: i32,
    /// The opponent's most recent ping measurement, or 0 if there hasn't been one yet.
    pub ping_us: u64,
}

/// Starts tracking a game. Call when the game's first online frame is handled.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_match_quality_start_game(config: SlippiMatchQualityGameConfig) {
    slippi_match_quality::start_game(GameConfig {
        can_terminate: config.can_terminate,
        delay: config.delay,
    });
}

/// Stops tracking the current game, if any.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_match_quality_end_game() {
    slippi_match_quality::end_game();
}

/// Reports a pass through the online frame handler, halted or not. Returns true once, when the match should be
/// terminated for poor quality.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_match_quality_on_frame(frame: SlippiMatchQualityFrame) -> bool {
    slippi_match_quality::on_frame(Frame {
        frame: frame.frame,
        time_us: frame.time_us,
        is_stall: frame.is_stall,
        latest_remote_frame: frame.latest_remote_frame,
        ping_us: frame.ping_us,
    })
}
