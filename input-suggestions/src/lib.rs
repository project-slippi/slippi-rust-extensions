//! Input suggestions for Slippi Online.
//!
//! The game only reads the controller once per frame, but the GameCube adapter delivers new reports
//! several times per frame, and Dolphin can sample a Standard Controller several times per frame too.
//! Whenever a report or sample changes the local input by enough to matter, we send it to the opponent
//! right away as a *suggestion* tagged with the frame it will first apply to. A client that has to
//! predict an opponent's input uses the newest applicable suggestion instead of repeating the last input
//! it received.
//!
//! Suggestions only ever feed predictions. The ASM records every prediction and rolls back when the real
//! input differs, so the game always ends up running on the real inputs on every client.
//!
//! All state lives behind one mutex because it is reached from several threads: the adapter read thread
//! (`on_adapter_report`), whichever thread samples a Standard Controller (`on_controller_sample`), the
//! CPU thread (frames and predictions) and the netplay thread (inputs and suggestions received from
//! peers).

use std::sync::{LazyLock, Mutex, MutexGuard};

use dolphin_integrations::Log;

mod pad;
mod remote;

pub use pad::{ADAPTER_PORT_DATA_SIZE, AdapterPortData, Calibration, PAD_SIZE, Pad, PadStatus};
pub use remote::PREDICTED_FRAME_COUNT;

use pad::{START_SYNC_FRAME, adapter_status, is_meaningful_change, status_to_pad};
use remote::Remote;

/// What the controller port the local player's inputs come from is set to in Dolphin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSource {
    /// A GameCube adapter port. Its reports arrive on the adapter read thread (`on_adapter_report`).
    Adapter,
    /// A Standard Controller (keyboard, XInput, DirectInput, SDL and so on). Dolphin samples it between
    /// the game's polls (`on_controller_sample`).
    StandardController,
    /// Anything else. Nothing reports its input between polls, so no suggestions are produced.
    Other,
}

/// Settings for a game, gathered by the EXI device when the game starts.
#[derive(Clone, Debug)]
pub struct GameConfig {
    /// Send suggestions and use the opponent's for predictions.
    pub enabled: bool,
    pub local_player_idx: u8,
    pub remote_player_count: u8,
    pub delay: u8,
    /// The controller port the game reads the local player's inputs from.
    pub source_port: u8,
    pub source: InputSource,
}

/// Facts about a local frame, reported right after the game polled and sent its input.
#[derive(Clone, Copy, Debug)]
pub struct LocalFrame {
    pub frame: i32,
    /// The frame is being skipped for time sync, so the polled input was not sent and the same frame
    /// will be polled again.
    pub is_skip: bool,
    pub is_advancing: bool,
    /// How many times the game has polled the local controller port, this frame's poll included.
    pub poll_count: Option<u64>,
    pub calibration: Option<Calibration>,
    /// The input the game sent (or would have sent, on a skipped frame).
    pub sent_pad: Pad,
}

/// A suggestion to send to the opponents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The first frame (in the sender's frame numbering, delay included) the input applies to.
    pub target_frame: i32,
    pub seq: u32,
    pub pad: Pad,
}

/// An adapter report or controller sample taken after a game poll that has not been handled yet. It is for
/// the frame after that poll's, which is only known once the poll is handled.
#[derive(Clone, Copy, Debug)]
struct PendingSample {
    poll_count: u64,
    status: PadStatus,
}

struct Game {
    config: GameConfig,
    /// Suggestions are enabled and the local player's port reports its input between polls.
    sending: bool,
    calibration: Option<Calibration>,
    /// The game's poll count as of the frame most recently handled by `on_local_frame`.
    handled_poll_count: Option<u64>,
    /// The frame the next game poll will be sent as (before delay is added).
    next_poll_frame: i32,
    /// The newest input the opponents know about from us, as a real input or a suggestion.
    last_communicated: Option<Pad>,
    pending: Option<PendingSample>,
    next_seq: u32,
    remotes: Vec<Remote>,
}

impl Game {
    fn new(config: GameConfig) -> Self {
        let remotes = (0..config.remote_player_count)
            .map(|i| {
                let port = if i >= config.local_player_idx { i + 1 } else { i };
                Remote::new(port)
            })
            .collect();

        tracing::info!(
            target: Log::SlippiOnline,
            enabled = config.enabled,
            delay = config.delay,
            source_port = config.source_port + 1,
            source = ?config.source,
            "[InputSuggestions] Game started"
        );

        Self {
            sending: config.enabled && config.source != InputSource::Other,
            config,
            calibration: None,
            handled_poll_count: None,
            next_poll_frame: 1,
            last_communicated: None,
            pending: None,
            next_seq: 0,
            remotes,
        }
    }

    fn delay(&self) -> i32 {
        self.config.delay as i32
    }

    fn can_send(&self) -> bool {
        self.sending && self.calibration.is_some() && self.handled_poll_count.is_some()
    }

    /// Whether the caller should sample the Standard Controller and report it with `on_controller_sample`.
    fn wants_controller_samples(&self) -> bool {
        self.sending && self.config.source == InputSource::StandardController
    }

    fn transform(&self, status: &PadStatus) -> Option<Pad> {
        status_to_pad(status, self.calibration.as_ref()?)
    }

    /// Sends `pad` as a suggestion for the next polled frame if it says something the opponents don't
    /// know yet.
    fn suggest(&mut self, pad: Pad) -> Option<Suggestion> {
        // The ASM zeroes inputs polled before this frame, so there is nothing to suggest
        if self.next_poll_frame < START_SYNC_FRAME - self.delay() {
            return None;
        }

        if !is_meaningful_change(&self.last_communicated?, &pad) {
            return None;
        }

        self.next_seq += 1;
        self.last_communicated = Some(pad);

        Some(Suggestion {
            target_frame: self.next_poll_frame + self.delay(),
            seq: self.next_seq,
            pad,
        })
    }

    /// `poll_counts` are the game's poll counts of each port when the report was stored.
    fn on_adapter_report(&mut self, poll_counts: &[u64; 4], payload: &[u8]) -> Option<Suggestion> {
        if self.config.source != InputSource::Adapter {
            return None;
        }

        let port = self.config.source_port as usize;
        let start = 1 + ADAPTER_PORT_DATA_SIZE * port;
        let port_data: AdapterPortData = payload.get(start..start + ADAPTER_PORT_DATA_SIZE)?.try_into().ok()?;
        let poll_count = *poll_counts.get(port)?;
        self.on_sample(poll_count, adapter_status(&port_data))
    }

    /// `poll_count` is the game's poll count of the port when the sample was read.
    fn on_controller_sample(&mut self, poll_count: u64, status: PadStatus) -> Option<Suggestion> {
        if self.config.source != InputSource::StandardController {
            return None;
        }

        self.on_sample(poll_count, status)
    }

    /// Handles the local input read between game polls, from an adapter report or a controller sample.
    /// `poll_count` is how many game polls of the port had started when it was read; the next poll reads
    /// this input or a newer one.
    fn on_sample(&mut self, poll_count: u64, status: PadStatus) -> Option<Suggestion> {
        if !self.can_send() {
            return None;
        }

        let handled_count = self.handled_poll_count?;

        // A poll we already handled read this input or a newer one, and that input has been sent
        if poll_count < handled_count {
            return None;
        }

        // The game polled after this input was read, so it is for the frame after that poll's, which we
        // only learn once the poll is handled. Keep only the newest such input.
        if poll_count > handled_count {
            self.pending = Some(PendingSample { poll_count, status });
            return None;
        }

        let pad = self.transform(&status)?;
        self.suggest(pad)
    }

    fn on_local_frame(&mut self, info: LocalFrame) -> Option<Suggestion> {
        if let Some(calibration) = info.calibration {
            self.calibration = Some(calibration);
        }
        if info.poll_count.is_some() {
            self.handled_poll_count = info.poll_count;
        }

        // A skipped frame is polled again. An advanced frame runs on this frame's poll, so the next poll is
        // for the frame after it. Otherwise the next poll is for the following frame
        self.next_poll_frame = if info.is_skip {
            info.frame
        } else if info.is_advancing {
            info.frame + 2
        } else {
            info.frame + 1
        };
        if !info.is_skip {
            self.last_communicated = Some(info.sent_pad);
        }

        // Release an input read after this frame's poll, now that we know which frame that poll was
        let mut candidate = None;
        if let (Some(pending), Some(count)) = (self.pending.take(), self.handled_poll_count) {
            if pending.poll_count > count {
                // Yet another poll is in flight
                self.pending = Some(pending);
            } else if pending.poll_count == count {
                candidate = self.transform(&pending.status);
            }
        }

        // The skipped frame's input was never sent, so it is news if it differs from what was
        if candidate.is_none() && info.is_skip {
            candidate = Some(info.sent_pad);
        }

        match candidate {
            Some(pad) if self.can_send() => self.suggest(pad),
            _ => None,
        }
    }

    fn on_remote_input(&mut self, remote_idx: usize, frame: i32, pad: Pad) {
        let Some(remote) = self.remotes.get_mut(remote_idx) else {
            return;
        };

        if remote.on_real_input(frame, pad) {
            tracing::error!(
                target: Log::SlippiOnline,
                "[InputSuggestions] Ignoring P{}'s suggestions for the rest of the game from frame {frame}: they \
                 kept disagreeing with their real inputs",
                remote.port + 1
            );
        }
    }

    fn fill_predictions(
        &self,
        remote_idx: usize,
        latest_frame: i32,
        is_disconnected: bool,
        latest: &[u8; 12],
    ) -> [[u8; 12]; PREDICTED_FRAME_COUNT] {
        let mut out = [*latest; PREDICTED_FRAME_COUNT];
        if !self.config.enabled || is_disconnected {
            return out;
        }

        if let Some(remote) = self.remotes.get(remote_idx) {
            remote.fill_predictions(latest_frame, latest, &mut out);
        }

        out
    }
}

static GAME: LazyLock<Mutex<Option<Game>>> = LazyLock::new(|| Mutex::new(None));

fn lock() -> MutexGuard<'static, Option<Game>> {
    // Release builds abort on panic, so the lock can't actually be poisoned there
    GAME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Starts tracking a new game, replacing the previous one. Returns whether the caller should sample the
/// Standard Controller between game polls and report it with `on_controller_sample` for this game.
pub fn start_game(config: GameConfig) -> bool {
    let game = Game::new(config);
    let wants_controller_samples = game.wants_controller_samples();
    *lock() = Some(game);
    wants_controller_samples
}

/// Stops tracking the current game, so nothing more is sent for it.
pub fn end_game() {
    *lock() = None;
}

/// Called on the CPU thread after the game polled the local controller and sent its input for a frame.
/// May return a suggestion that the caller must send.
pub fn on_local_frame(frame: LocalFrame) -> Option<Suggestion> {
    lock().as_mut()?.on_local_frame(frame)
}

/// Called on the adapter read thread for each report. May return a suggestion that the caller must send.
pub fn on_adapter_report(poll_counts: &[u64; 4], payload: &[u8]) -> Option<Suggestion> {
    lock().as_mut()?.on_adapter_report(poll_counts, payload)
}

/// Called for each sample of the Standard Controller, from any thread. `poll_count` is how many game polls
/// of the port had started when `status` was read, and the next game poll must read `status` or a newer
/// input. May return a suggestion that the caller must send.
pub fn on_controller_sample(poll_count: u64, status: PadStatus) -> Option<Suggestion> {
    lock().as_mut()?.on_controller_sample(poll_count, status)
}

pub fn on_remote_suggestion(remote_idx: usize, target_frame: i32, seq: u32, pad: Pad) {
    if let Some(remote) = lock().as_mut().and_then(|game| game.remotes.get_mut(remote_idx)) {
        remote.on_suggestion(target_frame, seq, pad);
    }
}

pub fn on_remote_input(remote_idx: usize, frame: i32, pad: Pad) {
    if let Some(game) = lock().as_mut() {
        game.on_remote_input(remote_idx, frame, pad);
    }
}

/// Forgets a remote player's suggestions and inputs, for when they start a new game.
pub fn reset_remote(remote_idx: usize) {
    if let Some(remote) = lock().as_mut().and_then(|game| game.remotes.get_mut(remote_idx)) {
        *remote = Remote::new(remote.port);
    }
}

/// Builds the inputs the ASM should use for each frame after `latest_frame` if it has to predict them.
/// `latest` is the latest input received from the player, which is what the ASM predicted before
/// suggestions existed and is what every slot holds when no suggestion applies.
pub fn fill_predictions(
    remote_idx: usize,
    latest_frame: i32,
    is_disconnected: bool,
    latest: &[u8; 12],
) -> [[u8; 12]; PREDICTED_FRAME_COUNT] {
    match lock().as_ref() {
        Some(game) => game.fill_predictions(remote_idx, latest_frame, is_disconnected, latest),
        None => [*latest; PREDICTED_FRAME_COUNT],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::adapter_to_pad;

    fn config() -> GameConfig {
        GameConfig {
            enabled: true,
            local_player_idx: 0,
            remote_player_count: 1,
            delay: 2,
            source_port: 0,
            source: InputSource::Adapter,
        }
    }

    fn controller_config() -> GameConfig {
        GameConfig {
            source: InputSource::StandardController,
            ..config()
        }
    }

    /// What a Standard Controller with the same input as `port_data(stick_x, a)` reports.
    fn controller_status(stick_x: u8, a: bool) -> PadStatus {
        adapter_status(&port_data(stick_x, a))
    }

    const CALIBRATION: Calibration = Calibration {
        origin: [128, 128, 128, 128, 0, 0],
        analog_mode: 3,
    };

    fn port_data(stick_x: u8, a: bool) -> AdapterPortData {
        [0x10, a as u8, 0, stick_x, 128, 128, 128, 0, 0]
    }

    fn payload(port: AdapterPortData) -> [u8; 37] {
        let mut p = [0; 37];
        p[0] = 0x21;
        p[1..10].copy_from_slice(&port);
        p
    }

    fn local_frame(frame: i32, poll_count: u64, data: AdapterPortData) -> LocalFrame {
        LocalFrame {
            frame,
            is_skip: false,
            is_advancing: false,
            poll_count: Some(poll_count),
            calibration: Some(CALIBRATION),
            sent_pad: adapter_to_pad(&data, &CALIBRATION).unwrap(),
        }
    }

    fn frame(game: &mut Game, frame: i32, poll_count: u64, data: AdapterPortData, skip: bool) -> Option<Suggestion> {
        game.on_local_frame(LocalFrame {
            is_skip: skip,
            ..local_frame(frame, poll_count, data)
        })
    }

    #[test]
    fn change_between_polls_targets_next_frame() {
        let mut game = Game::new(config());
        assert_eq!(frame(&mut game, 100, 1, port_data(128, false), false), None);

        // Noise is not sent
        assert_eq!(game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(129, false))), None);

        let s = game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))).unwrap();
        assert_eq!(s.target_frame, 103);
        assert_eq!(s.seq, 1);

        // The same input again says nothing new
        assert_eq!(game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))), None);
    }

    /// A report stored at the same poll count as the last handled poll is for the next frame (see
    /// `change_between_polls_targets_next_frame`). An older count means a later poll already read it, and a
    /// newer one means a poll is in flight that has to be handled first.
    #[test]
    fn reports_from_other_polls_wait_for_or_skip_them() {
        // Poll 5 is handled, so a report stored at count 4 was read by it and its input already sent
        let mut game = Game::new(config());
        frame(&mut game, 100, 5, port_data(128, false), false);
        assert_eq!(game.on_adapter_report(&[4, 0, 0, 0], &payload(port_data(200, false))), None);

        // This report was stored after poll 2, which hasn't been handled yet, so it waits for that poll
        let mut game = Game::new(config());
        frame(&mut game, 100, 1, port_data(128, false), false);
        assert_eq!(game.on_adapter_report(&[2, 0, 0, 0], &payload(port_data(200, false))), None);
        // Once poll 2 is handled as frame 101, the report is for frame 102 (+2 delay)
        let s = frame(&mut game, 101, 2, port_data(128, false), false).unwrap();
        assert_eq!(s.target_frame, 104);

        // A report stored after poll 3 waits through poll 2 as well
        let mut game = Game::new(config());
        frame(&mut game, 100, 1, port_data(128, false), false);
        game.on_adapter_report(&[3, 0, 0, 0], &payload(port_data(200, false)));
        assert_eq!(frame(&mut game, 101, 2, port_data(128, false), false), None);
        let s = frame(&mut game, 102, 3, port_data(128, false), false).unwrap();
        assert_eq!(s.target_frame, 105);
    }

    #[test]
    fn skipped_frame_input_is_suggested_for_the_same_frame() {
        let mut game = Game::new(config());
        frame(&mut game, 100, 1, port_data(128, false), false);
        let s = frame(&mut game, 101, 2, port_data(128, true), true).unwrap();
        // The frame is polled again, so the input applies from 101 + delay
        assert_eq!(s.target_frame, 103);
    }

    /// On a time sync advance the game runs the next frame right away on the same poll, so a change made
    /// after that poll is first read for the frame after the advanced one.
    #[test]
    fn change_during_advance_targets_frame_after_advanced_one() {
        let neutral = port_data(128, false);
        let advancing = |frame, poll_count| LocalFrame {
            is_advancing: true,
            ..local_frame(frame, poll_count, neutral)
        };

        // Stored after poll 1 but before frame 100 was handled
        let mut game = Game::new(config());
        frame(&mut game, 99, 0, neutral, false);
        assert_eq!(game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))), None);
        let s = game.on_local_frame(advancing(100, 1)).unwrap();
        assert_eq!(s.target_frame, 104);

        // Stored after frame 100 was handled
        let mut game = Game::new(config());
        game.on_local_frame(advancing(100, 1));
        let s = game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))).unwrap();
        assert_eq!(s.target_frame, 104);

        // The advanced frame 101 doesn't poll, and the next poll is still frame 102's
        assert_eq!(frame(&mut game, 101, 1, neutral, false), None);
        let s = game
            .on_adapter_report(&[1, 0, 0, 0], &payload(port_data(200, false)))
            .unwrap();
        assert_eq!(s.target_frame, 104);
    }

    #[test]
    fn nothing_is_sent_for_frozen_frames() {
        let mut game = Game::new(config());
        frame(&mut game, 10, 1, port_data(128, false), false);
        assert_eq!(game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))), None);
    }

    #[test]
    fn nothing_is_sent_when_disabled_or_off_the_adapter() {
        for config in [
            GameConfig {
                enabled: false,
                ..config()
            },
            GameConfig {
                source: InputSource::Other,
                ..config()
            },
            controller_config(),
        ] {
            let mut game = Game::new(config);
            frame(&mut game, 100, 1, port_data(128, false), false);
            assert_eq!(game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))), None);
        }
    }

    /// A Standard Controller sample follows the same frame rules as an adapter report: the poll count it
    /// was read at decides which frame it is for.
    #[test]
    fn controller_samples_target_frames_like_adapter_reports() {
        let mut game = Game::new(controller_config());
        frame(&mut game, 100, 1, port_data(128, false), false);

        // Read after poll 1, which was frame 100, so it first applies to 101 + delay
        let s = game.on_controller_sample(1, controller_status(128, true)).unwrap();
        assert_eq!(s.target_frame, 103);

        // Read after poll 2, which hasn't been handled yet, so it waits for that poll
        assert_eq!(game.on_controller_sample(2, controller_status(200, true)), None);
        let s = frame(&mut game, 101, 2, port_data(128, true), false).unwrap();
        assert_eq!(s.target_frame, 104);

        // Read before poll 3 was handled as frame 102: that poll read it, and its input was sent
        frame(&mut game, 102, 3, port_data(128, false), false);
        assert_eq!(game.on_controller_sample(2, controller_status(128, true)), None);
    }

    #[test]
    fn controller_samples_are_only_wanted_from_a_standard_controller() {
        assert!(Game::new(controller_config()).wants_controller_samples());
        assert!(!Game::new(config()).wants_controller_samples());
        assert!(
            !Game::new(GameConfig {
                enabled: false,
                ..controller_config()
            })
            .wants_controller_samples()
        );

        // Samples are ignored when the port is an adapter port
        let mut game = Game::new(config());
        frame(&mut game, 100, 1, port_data(128, false), false);
        assert_eq!(game.on_controller_sample(1, controller_status(128, true)), None);
    }

    #[test]
    fn suggestions_are_only_used_when_enabled_and_connected() {
        let first_slot = |enabled: bool, is_disconnected: bool| {
            let mut game = Game::new(GameConfig { enabled, ..config() });
            game.remotes[0].on_real_input(100, [0; 8]);
            game.remotes[0].on_suggestion(101, 1, [0x01, 0, 0, 0, 0, 0, 0, 0]);
            game.fill_predictions(0, 100, is_disconnected, &[0; 12])[0][0]
        };

        assert_eq!(first_slot(true, false), 0x01);
        // Otherwise every slot holds the latest input, as before suggestions existed
        assert_eq!(first_slot(false, false), 0);
        assert_eq!(first_slot(true, true), 0);
    }

    /// A sender and a receiver that is 3 frames behind on the sender's real inputs: a button press made
    /// between polls reaches the receiver as a suggestion and is predicted correctly.
    #[test]
    fn suggestion_reaches_receiver_prediction() {
        let mut sender = Game::new(config());
        let mut receiver = Game::new(GameConfig {
            local_player_idx: 1,
            ..config()
        });

        let neutral = adapter_to_pad(&port_data(128, false), &CALIBRATION).unwrap();
        let pressed = adapter_to_pad(&port_data(128, true), &CALIBRATION).unwrap();

        frame(&mut sender, 100, 1, port_data(128, false), false);
        receiver.on_remote_input(0, 100, neutral);

        // A pressed between polls 100 and 101, so it first applies to 101 + delay = 103
        let s = sender
            .on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true)))
            .unwrap();
        receiver.remotes[0].on_suggestion(s.target_frame, s.seq, s.pad);

        // The receiver has real inputs up to 100 and must predict 101..=103
        let mut latest = [0; 12];
        latest[..8].copy_from_slice(&neutral);
        let out = receiver.fill_predictions(0, 100, false, &latest);
        assert_eq!(out[0][..8], neutral);
        assert_eq!(out[1][..8], neutral);
        assert_eq!(out[2][..8], pressed);
    }
}
