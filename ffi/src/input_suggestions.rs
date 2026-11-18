//! C FFI for input suggestions. See the `slippi-input-suggestions` crate for how they work.
//!
//! These functions are called from several Dolphin threads (the adapter read thread, the thread that
//! samples a Standard Controller, the CPU thread and the netplay thread); the crate serializes them
//! internally.

use std::slice;

use slippi_input_suggestions::{
    Calibration, GameConfig, InputSource, LocalFrame, PAD_SIZE, PREDICTED_FRAME_COUNT, PadStatus, Suggestion,
};

/// A suggestion the caller should send to the opponents. Ignore it unless `is_valid` is set.
#[repr(C)]
pub struct SlippiInputSuggestion {
    pub is_valid: bool,
    pub target_frame: i32,
    pub seq: u32,
    pub pad: [u8; 8],
}

impl From<Option<Suggestion>> for SlippiInputSuggestion {
    fn from(suggestion: Option<Suggestion>) -> Self {
        match suggestion {
            Some(s) => Self {
                is_valid: true,
                target_frame: s.target_frame,
                seq: s.seq,
                pad: s.pad,
            },
            None => Self {
                is_valid: false,
                target_frame: 0,
                seq: 0,
                pad: [0; 8],
            },
        }
    }
}

/// Settings for a game, passed when it starts.
#[repr(C)]
pub struct SlippiInputSuggestionsGameConfig {
    pub enabled: bool,
    pub local_player_idx: u8,
    pub remote_player_count: u8,
    pub delay: u8,
    pub source_port: u8,
    /// The source port is a GameCube adapter port.
    pub source_is_adapter: bool,
    /// The source port is a Standard Controller.
    pub source_is_standard_controller: bool,
}

/// The Standard Controller's input, read between game polls.
#[repr(C)]
pub struct SlippiInputSuggestionsControllerSample {
    /// How many game polls of the port had started when the input was read. The game's next poll of the
    /// port must read this input or a newer one.
    pub game_poll_count: u64,
    /// `GCPadStatus::button`.
    pub button: u16,
    /// Stick x/y, c-stick x/y, L, R from the `GCPadStatus`.
    pub analog: [u8; 6],
}

/// What happened on a local frame: how many times the game has polled the controller port and what it sent.
#[repr(C)]
pub struct SlippiInputSuggestionsLocalFrame {
    pub frame: i32,
    pub is_skip: bool,
    pub is_advancing: bool,
    pub has_poll: bool,
    pub poll_count: u64,
    pub has_origin: bool,
    /// Stick x/y, c-stick x/y, L, R as reported to the game.
    pub origin: [u8; 6],
    pub analog_mode: u8,
    pub sent_pad: [u8; 8],
}

/// Starts tracking a game. Call on the CPU thread when the game's first online frame is handled.
///
/// Returns whether the caller should sample the Standard Controller on the source port between game polls
/// for this game, and report each sample with `slprs_input_suggestions_on_controller_sample`.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_start_game(config: SlippiInputSuggestionsGameConfig) -> bool {
    let source = if config.source_is_adapter {
        InputSource::Adapter
    } else if config.source_is_standard_controller {
        InputSource::StandardController
    } else {
        InputSource::Other
    };

    slippi_input_suggestions::start_game(GameConfig {
        enabled: config.enabled,
        local_player_idx: config.local_player_idx,
        remote_player_count: config.remote_player_count,
        delay: config.delay,
        source_port: config.source_port,
        source,
    })
}

/// Stops tracking the current game, if any.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_end_game() {
    slippi_input_suggestions::end_game();
}

/// Reports a local frame. Call on the CPU thread right after the frame's input was sent and Slippi decided
/// whether to advance. Send the returned suggestion if it is valid.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_on_local_frame(frame: SlippiInputSuggestionsLocalFrame) -> SlippiInputSuggestion {
    let calibration = frame.has_origin.then_some(Calibration {
        origin: frame.origin,
        analog_mode: frame.analog_mode,
    });

    slippi_input_suggestions::on_local_frame(LocalFrame {
        frame: frame.frame,
        is_skip: frame.is_skip,
        is_advancing: frame.is_advancing,
        poll_count: frame.has_poll.then_some(frame.poll_count),
        calibration,
        sent_pad: frame.sent_pad,
    })
    .into()
}

/// Reports an adapter input report. Call on the adapter read thread. `game_poll_counts` points at four
/// counts, one per port. Send the returned suggestion if it is valid.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_on_adapter_report(
    game_poll_counts: *const u64,
    payload: *const u8,
    payload_len: usize,
) -> SlippiInputSuggestion {
    if game_poll_counts.is_null() || payload.is_null() {
        return None.into();
    }

    // The C++ side owns both buffers for the duration of the call.
    let (poll_counts, payload) = unsafe {
        (
            &*(game_poll_counts as *const [u64; 4]),
            slice::from_raw_parts(payload, payload_len),
        )
    };

    slippi_input_suggestions::on_adapter_report(poll_counts, payload).into()
}

/// Reports a sample of the Standard Controller on the source port. Call from any thread, only while
/// `slprs_input_suggestions_start_game` asked for samples. Send the returned suggestion if it is valid.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_on_controller_sample(
    sample: SlippiInputSuggestionsControllerSample,
) -> SlippiInputSuggestion {
    let status = PadStatus {
        button: sample.button,
        analog: sample.analog,
    };

    slippi_input_suggestions::on_controller_sample(sample.game_poll_count, status).into()
}

/// Reports a suggestion received from a remote player. `pad` points at 8 bytes. Call on the netplay thread.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_on_remote_suggestion(remote_idx: u8, target_frame: i32, seq: u32, pad: *const u8) {
    if pad.is_null() {
        return;
    }

    // The C++ side owns the buffer for the duration of the call.
    let pad = unsafe { *(pad as *const [u8; PAD_SIZE]) };
    slippi_input_suggestions::on_remote_suggestion(remote_idx as usize, target_frame, seq, pad);
}

/// Reports a real input received from a remote player. `pad` points at 8 bytes. Call on the netplay thread.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_on_remote_input(remote_idx: u8, frame: i32, pad: *const u8) {
    if pad.is_null() {
        return;
    }

    // The C++ side owns the buffer for the duration of the call.
    let pad = unsafe { *(pad as *const [u8; PAD_SIZE]) };
    slippi_input_suggestions::on_remote_input(remote_idx as usize, frame, pad);
}

/// Forgets a remote player's suggestions and inputs, for when they start a new game.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_reset_remote(remote_idx: u8) {
    slippi_input_suggestions::reset_remote(remote_idx as usize);
}

/// Writes the input the ASM should use for each of the 7 frames after `latest_frame` into `out`
/// (7 pad reports of 12 bytes). `latest_pad` points at the 12-byte latest received input, which every
/// slot holds unless a suggestion applies. Call on the CPU thread.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_input_suggestions_fill_predictions(
    remote_idx: u8,
    latest_frame: i32,
    is_disconnected: bool,
    latest_pad: *const u8,
    out: *mut u8,
    out_len: usize,
) {
    if latest_pad.is_null() || out.is_null() || out_len < PREDICTED_FRAME_COUNT * 12 {
        return;
    }

    // The C++ side owns both buffers for the duration of the call.
    let (latest, out) = unsafe { (&*(latest_pad as *const [u8; 12]), slice::from_raw_parts_mut(out, out_len)) };

    let predictions = slippi_input_suggestions::fill_predictions(remote_idx as usize, latest_frame, is_disconnected, latest);

    for (chunk, prediction) in out.chunks_exact_mut(12).zip(predictions.iter()) {
        chunk.copy_from_slice(prediction);
    }
}
