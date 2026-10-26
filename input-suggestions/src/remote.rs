//! Tracks one remote player's suggestions, builds the predictions handed to the ASM from them, and stops
//! using them if they keep disagreeing with the player's real inputs.

use std::collections::VecDeque;

use crate::pad::{Pad, analog_error, wrong_buttons};

/// The number of frames after a player's latest received frame that the ASM can ask a prediction for
/// (`RXB_INPUTS_COUNT`, which is `ROLLBACK_MAX_FRAME_COUNT`).
pub const PREDICTED_FRAME_COUNT: usize = 7;

/// Suggestions for frames further than this past the latest real input are treated as garbage, for
/// instance packets from a previous game.
const MAX_SUGGESTION_LEAD_FRAMES: i32 = 120;

/// Caps memory if a peer floods suggestions.
const MAX_STORED_SUGGESTIONS: usize = 512;

/// A player whose suggestions don't match their real inputs only makes the game look worse. Each suggestion
/// is checked once, against the real input for the frame it targets, and a mistake the player's previous
/// input didn't make adds its cost to a bucket. Checking only at the target frame keeps lag from counting
/// an honest suggestion against every frame predicted from it. The bucket drains one point every
/// `FRAMES_PER_DRAINED_POINT` frames, so occasional misses never add up however long the game runs. If it
/// goes over `IGNORE_AFTER_POINTS`, the player's suggestions are ignored for the rest of the game.
const IGNORE_AFTER_POINTS: u32 = 15;
const FRAMES_PER_DRAINED_POINT: u32 = 200;
/// A wrong button the previous input had right.
const WRONG_BUTTON_COST: u32 = 3;
/// A stick or trigger further off than the previous input, with the buttons no better or worse.
const FARTHER_ANALOG_COST: u32 = 1;

#[derive(Clone, Copy, Debug)]
struct RemoteSuggestion {
    target_frame: i32,
    seq: u32,
    pad: Pad,
}

#[derive(Debug)]
pub struct Remote {
    pub port: u8,
    suggestions: VecDeque<RemoteSuggestion>,
    /// The newest real input received, and its frame.
    latest_real: Option<(i32, Pad)>,
    /// See `IGNORE_AFTER_POINTS`, in frames' worth of draining.
    worse_bucket: u32,
    ignored: bool,
}

impl Remote {
    pub fn new(port: u8) -> Self {
        Self {
            port,
            suggestions: VecDeque::new(),
            latest_real: None,
            worse_bucket: 0,
            ignored: false,
        }
    }

    pub fn on_suggestion(&mut self, target_frame: i32, seq: u32, pad: Pad) {
        let latest = self.latest_real.map_or(0, |(frame, _)| frame);
        if self.ignored || target_frame <= latest || target_frame > latest + MAX_SUGGESTION_LEAD_FRAMES {
            return;
        }

        if self.suggestions.len() >= MAX_STORED_SUGGESTIONS {
            self.suggestions.pop_front();
        }

        self.suggestions.push_back(RemoteSuggestion { target_frame, seq, pad });
    }

    /// The newest suggestion that applies to `frame` when the latest real input is for `latest_frame`.
    /// A suggestion for frame T describes the input from T onward, so it applies when T <= frame, and
    /// it is only newer than the real inputs when T > latest_frame.
    fn best_suggestion(&self, latest_frame: i32, frame: i32) -> Option<&RemoteSuggestion> {
        self.suggestions
            .iter()
            .filter(|s| s.target_frame > latest_frame && s.target_frame <= frame)
            .max_by_key(|s| s.seq)
    }

    /// Fills `out` with the input the ASM should use for each of the frames after `latest_frame`: the
    /// newest applicable suggestion, or `latest` (the latest received input) when there is none.
    pub fn fill_predictions(&self, latest_frame: i32, latest: &[u8; 12], out: &mut [[u8; 12]; PREDICTED_FRAME_COUNT]) {
        for (k, slot) in out.iter_mut().enumerate() {
            *slot = *latest;
            if let Some(suggestion) = self.best_suggestion(latest_frame, latest_frame + 1 + k as i32) {
                // Remote pads only ever carry 8 bytes; the rest of the report is zero
                *slot = [0; 12];
                slot[..8].copy_from_slice(&suggestion.pad);
            }
        }
    }

    /// Returns true when this input is the one that got the player's suggestions ignored.
    pub fn on_real_input(&mut self, frame: i32, pad: Pad) -> bool {
        let previous = self.latest_real;
        if previous.is_some_and(|(latest, _)| frame <= latest) {
            return false;
        }
        self.latest_real = Some((frame, pad));

        // Check the suggestion in effect for this frame against what the player actually did
        self.worse_bucket = self.worse_bucket.saturating_sub(1);
        let suggested = self.best_suggestion(frame - 1, frame).map(|s| s.pad);
        if let (Some(suggested), Some((previous_frame, previous_pad))) = (suggested, previous) {
            if previous_frame == frame - 1 {
                self.worse_bucket += worse_cost(&previous_pad, &suggested, &pad) * FRAMES_PER_DRAINED_POINT;
            }
        }

        // Nothing at or before the latest real input is needed for predictions any more
        self.suggestions.retain(|s| s.target_frame > frame);

        if self.ignored || self.worse_bucket <= IGNORE_AFTER_POINTS * FRAMES_PER_DRAINED_POINT {
            return false;
        }

        self.ignored = true;
        self.suggestions.clear();
        true
    }
}

/// What a suggestion costs for being further from the real input than the player's previous input was.
fn worse_cost(previous: &Pad, suggested: &Pad, real: &Pad) -> u32 {
    let previous_wrong = wrong_buttons(previous, real);
    let suggested_wrong = wrong_buttons(suggested, real);
    if suggested_wrong & !previous_wrong != 0 {
        WRONG_BUTTON_COST
    } else if suggested_wrong == previous_wrong && analog_error(suggested, real) > analog_error(previous, real) {
        FARTHER_ANALOG_COST
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(first: u8) -> Pad {
        [first, 0, 0, 0, 0, 0, 0, 0]
    }

    fn report(p: Pad) -> [u8; 12] {
        let mut r = [0; 12];
        r[..8].copy_from_slice(&p);
        r
    }

    #[test]
    fn newest_applicable_suggestion_fills_each_slot() {
        let mut remote = Remote::new(1);
        remote.on_real_input(100, pad(0));
        remote.on_suggestion(102, 1, pad(1));
        remote.on_suggestion(104, 2, pad(2));
        // Arrives out of order but is older
        remote.on_suggestion(102, 0, pad(9));

        let mut out = [[0; 12]; PREDICTED_FRAME_COUNT];
        remote.fill_predictions(100, &report(pad(0)), &mut out);

        let firsts: Vec<u8> = out.iter().map(|s| s[0]).collect();
        // Frames 101..=107
        assert_eq!(firsts, vec![0, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn late_and_stale_suggestions_are_rejected() {
        let mut remote = Remote::new(1);
        remote.on_real_input(100, pad(0));
        remote.on_suggestion(100, 1, pad(1));
        remote.on_suggestion(5000, 2, pad(1));
        assert!(remote.suggestions.is_empty());
    }

    /// Suggests `suggested` for `frame`, then delivers the real input for it. The previous real input is
    /// always neutral in these tests.
    fn suggest_and_receive(remote: &mut Remote, frame: i32, suggested: Pad, real: Pad) -> bool {
        remote.on_suggestion(frame, frame as u32, suggested);
        remote.on_real_input(frame, real)
    }

    /// Suggests a press that never happens on each of `frames`, returning the frames that started ignoring.
    fn suggest_wrong_presses(remote: &mut Remote, frames: std::ops::RangeInclusive<i32>) -> Vec<i32> {
        frames
            .filter(|&frame| suggest_and_receive(remote, frame, pad(1), pad(0)))
            .collect()
    }

    #[test]
    fn worse_cost_counts_only_what_the_suggestion_made_worse() {
        let real_a = [0x01, 0, 10, 0, 0, 0, 0, 0];
        let nothing = [0x00, 0, 10, 0, 0, 0, 0, 0];

        // Fixing a button outweighs a farther stick
        assert_eq!(worse_cost(&nothing, &[0x01, 0, 40, 0, 0, 0, 0, 0], &real_a), 0);
        // B instead of A is worse than nothing, even though both miss the A
        assert_eq!(
            worse_cost(&nothing, &[0x02, 0, 10, 0, 0, 0, 0, 0], &real_a),
            WRONG_BUTTON_COST
        );
        // Same buttons, farther or closer stick
        assert_eq!(
            worse_cost(&real_a, &[0x01, 0, 30, 0, 0, 0, 0, 0], &real_a),
            FARTHER_ANALOG_COST
        );
        assert_eq!(
            worse_cost(&[0x01, 0, 30, 0, 0, 0, 0, 0], &[0x01, 0, 12, 0, 0, 0, 0, 0], &real_a),
            0
        );
    }

    #[test]
    fn suggestions_are_judged_only_at_their_target_frame() {
        let mut remote = Remote::new(1);
        remote.on_real_input(100, pad(0));

        // A tapped for frames 101 and 102 with the release never suggested. During a lag spike every frame up
        // to the real inputs is predicted with A held, but the suggestion was true for the frame it targeted.
        remote.on_suggestion(101, 1, pad(1));
        let mut out = [[0; 12]; PREDICTED_FRAME_COUNT];
        remote.fill_predictions(100, &report(pad(0)), &mut out);
        assert!(out.iter().all(|slot| slot[0] == 1));
        for (frame, real) in [(101, pad(1)), (102, pad(1)), (103, pad(0)), (104, pad(0)), (105, pad(0))] {
            remote.on_real_input(frame, real);
        }

        // Only the newest suggestion for a frame counts: A pressed and released again before the poll
        remote.on_suggestion(106, 2, pad(1));
        remote.on_suggestion(106, 3, pad(0));
        remote.on_real_input(106, pad(0));

        assert_eq!(remote.worse_bucket, 0);
    }

    #[test]
    fn wrong_suggestions_are_ignored_past_the_limit() {
        let mut remote = Remote::new(1);
        remote.on_real_input(100, pad(0));

        assert_eq!(
            suggest_wrong_presses(&mut remote, 101..=110),
            vec![106],
            "the sixth wrong press passes 15"
        );

        // Later suggestions are dropped, so predictions fall back to the latest input
        remote.on_suggestion(112, 1000, pad(1));
        assert!(remote.suggestions.is_empty());
        let mut out = [[0; 12]; PREDICTED_FRAME_COUNT];
        remote.fill_predictions(110, &report(pad(0)), &mut out);
        assert!(out.iter().all(|slot| slot[0] == 0));
    }

    #[test]
    fn occasional_misses_never_add_up_in_a_long_game() {
        let mut remote = Remote::new(1);
        remote.on_real_input(100, pad(0));

        // Twice the worst rate an honest game showed, for about half an hour of frames: a farther stick every
        // 400 frames and a wrong button every 2000
        let mut peak = 0;
        for frame in 101..=100_100 {
            let mut suggested = pad(0);
            if frame % 400 == 0 {
                suggested[2] = 5;
            }
            if frame % 2000 == 1000 {
                suggested[0] = 1;
            }
            assert!(!suggest_and_receive(&mut remote, frame, suggested, pad(0)));
            peak = peak.max(remote.worse_bucket);
        }
        assert!(peak < 5 * FRAMES_PER_DRAINED_POINT, "peak {peak}");
    }

    #[test]
    fn a_clean_stretch_does_not_bank_allowance() {
        let mut remote = Remote::new(1);
        remote.on_real_input(100, pad(0));

        for frame in 101..=20_100 {
            suggest_and_receive(&mut remote, frame, pad(0), pad(0));
        }
        assert_eq!(suggest_wrong_presses(&mut remote, 20_101..=20_110), vec![20_106]);
    }
}
