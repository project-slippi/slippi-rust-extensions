use crate::errors::EngineError;
use crate::types::{FrameDecision, FrameInputs, FramePackage, StartConditions};
use std::path::PathBuf;

#[derive(Default, Debug)]
pub struct PlaybackState {
    pub(crate) current_replay_path: Option<PathBuf>,
    frames: Vec<FrameInputs>,
    next_frame: usize,
    start_conditions: Option<StartConditions>,
    rng_seed: u32,

    // Prepared Gecko data
    gecko_blob: Option<Vec<u8>>, // raw bytes to hand to the game
    gecko_size: usize,

    // Playback flags – feed these from your UI or external control surface.
    pub paused: bool,
    pub fast_forward: bool,
    pub should_terminate: bool,
}

impl PlaybackState {
    pub fn reset_for_new_game(&mut self, new_path: PathBuf, _parsed: &crate::parser::ParsedReplay) {
        self.current_replay_path = Some(new_path);
        self.frames.clear();
        self.next_frame = 0;
        self.start_conditions = None;
        self.rng_seed = 0;
        self.gecko_blob = None;
        self.gecko_size = 0;
        self.paused = false;
        self.fast_forward = false;
        self.should_terminate = false;
    }

    pub fn store_start_conditions(&mut self, sc: StartConditions) {
        self.start_conditions = Some(sc);
    }

    pub fn store_frames(&mut self, frames: Vec<FrameInputs>) {
        self.frames = frames;
    }

    pub fn store_initial_rng(&mut self, seed: u32) {
        self.rng_seed = seed;
    }

    pub fn has_minimum_start_data(&self) -> bool {
        self.start_conditions.is_some() && !self.frames.is_empty()
    }

    pub fn start_conditions(&self) -> Option<&StartConditions> {
        self.start_conditions.as_ref()
    }

    pub fn store_gecko_codes(&mut self, bytes: Vec<u8>, total_size: usize) {
        self.gecko_blob = Some(bytes);
        self.gecko_size = total_size;
    }

    pub fn gecko_blob(&self) -> Option<(&[u8], usize)> {
        self.gecko_blob.as_deref().map(|b| (b, self.gecko_size))
    }

    pub fn compute_frame_decision(&self) -> FrameDecision {
        if self.should_terminate {
            return FrameDecision::Terminate;
        }
        if self.paused {
            return FrameDecision::Halt;
        }
        if self.fast_forward {
            return FrameDecision::FastForward;
        }
        FrameDecision::Play
    }

    pub fn next_frame_package(&mut self) -> Result<Option<FramePackage>, EngineError> {
        if self.next_frame >= self.frames.len() {
            return Ok(None); // No more frames; caller can decide to end the game.
        }
        let idx = self.next_frame;
        let inputs = self.frames[idx].clone();
        // HINT: If RNG should advance per-frame, mutate `self.rng_seed` here.
        let pkg = FramePackage {
            frame_index: idx,
            inputs,
            rng_seed: self.rng_seed,
        };
        self.next_frame += 1;
        Ok(Some(pkg))
    }
}
