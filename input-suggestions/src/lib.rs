//! Input suggestions for Slippi Online.
//!
//! The game only reads the controller once per frame, but the GameCube adapter delivers new reports
//! several times per frame. Whenever a report changes the local input by enough to matter, we send it to
//! the opponent right away as a *suggestion* tagged with the frame it will first apply to. A client that
//! has to predict an opponent's input uses the newest applicable suggestion instead of repeating the last
//! input it received.
//!
//! Suggestions only ever feed predictions. The ASM records every prediction and rolls back when the real
//! input differs, so the game always ends up running on the real inputs on every client.
//!
//! All state lives behind one mutex because it is reached from three threads: the adapter read thread
//! (`on_adapter_report`), the CPU thread (frames and predictions) and the netplay thread (inputs and
//! suggestions received from peers).

use std::sync::{LazyLock, Mutex, MutexGuard};

use dolphin_integrations::Log;

mod pad;
mod remote;

pub use pad::{ADAPTER_PORT_DATA_SIZE, AdapterPortData, Calibration, PAD_SIZE, Pad};
pub use remote::PREDICTED_FRAME_COUNT;

use pad::{START_SYNC_FRAME, adapter_to_pad, is_meaningful_change};
use remote::Remote;

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
    /// Whether that port is a GameCube adapter port. Suggestions are only produced for the adapter.
    pub source_is_adapter: bool,
}

/// Facts about a local frame, reported right after the game polled and sent its input.
#[derive(Clone, Copy, Debug)]
pub struct LocalFrame {
    pub frame: i32,
    /// The frame is being skipped for time sync, so the polled input was not sent and the same frame
    /// will be polled again.
    pub is_skip: bool,
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

/// A report stored after a game poll that has not been handled yet. It is for the frame after that poll's,
/// which is only known once the poll is handled.
#[derive(Clone, Copy, Debug)]
struct PendingReport {
    poll_count: u64,
    port_data: AdapterPortData,
}

struct Game {
    config: GameConfig,
    /// Suggestions are enabled and the local player is on a GameCube adapter port.
    sending: bool,
    calibration: Option<Calibration>,
    /// The game's poll count as of the frame most recently handled by `on_local_frame`.
    handled_poll_count: Option<u64>,
    /// The frame the next game poll will be sent as (before delay is added).
    next_poll_frame: i32,
    /// The newest input the opponents know about from us, as a real input or a suggestion.
    last_communicated: Option<Pad>,
    pending: Option<PendingReport>,
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
            source_is_adapter = config.source_is_adapter,
            "[InputSuggestions] Game started"
        );

        Self {
            sending: config.enabled && config.source_is_adapter,
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

    fn transform(&self, port_data: &AdapterPortData) -> Option<Pad> {
        adapter_to_pad(port_data, self.calibration.as_ref()?)
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
        if !self.can_send() {
            return None;
        }

        let port = self.config.source_port as usize;
        let start = 1 + ADAPTER_PORT_DATA_SIZE * port;
        let port_data: AdapterPortData = payload.get(start..start + ADAPTER_PORT_DATA_SIZE)?.try_into().ok()?;
        let handled_count = self.handled_poll_count?;
        let poll_count = *poll_counts.get(port)?;

        // A poll we already handled read this report or a newer one, and that input has been sent
        if poll_count < handled_count {
            return None;
        }

        // The game polled after this report was stored, so the report is for the frame after that poll's,
        // which we only learn once the poll is handled. Keep only the newest such report.
        if poll_count > handled_count {
            self.pending = Some(PendingReport { poll_count, port_data });
            return None;
        }

        let pad = self.transform(&port_data)?;
        self.suggest(pad)
    }

    fn on_local_frame(&mut self, info: LocalFrame) -> Option<Suggestion> {
        if let Some(calibration) = info.calibration {
            self.calibration = Some(calibration);
        }
        if info.poll_count.is_some() {
            self.handled_poll_count = info.poll_count;
        }

        // A skipped frame is polled again; otherwise the next poll is for the following frame
        self.next_poll_frame = if info.is_skip { info.frame } else { info.frame + 1 };
        if !info.is_skip {
            self.last_communicated = Some(info.sent_pad);
        }

        // Release a report stored after this frame's poll, now that we know which frame that poll was
        let mut candidate = None;
        if let (Some(pending), Some(count)) = (self.pending.take(), self.handled_poll_count) {
            if pending.poll_count > count {
                // Yet another poll is in flight
                self.pending = Some(pending);
            } else if pending.poll_count == count {
                candidate = self.transform(&pending.port_data);
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

/// Starts tracking a new game, replacing the previous one.
pub fn start_game(config: GameConfig) {
    *lock() = Some(Game::new(config));
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

    fn config() -> GameConfig {
        GameConfig {
            enabled: true,
            local_player_idx: 0,
            remote_player_count: 1,
            delay: 2,
            source_port: 0,
            source_is_adapter: true,
        }
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

    fn frame(game: &mut Game, frame: i32, poll_count: u64, data: AdapterPortData, skip: bool) -> Option<Suggestion> {
        game.on_local_frame(LocalFrame {
            frame,
            is_skip: skip,
            poll_count: Some(poll_count),
            calibration: Some(CALIBRATION),
            sent_pad: adapter_to_pad(&data, &CALIBRATION).unwrap(),
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
                source_is_adapter: false,
                ..config()
            },
        ] {
            let mut game = Game::new(config);
            frame(&mut game, 100, 1, port_data(128, false), false);
            assert_eq!(game.on_adapter_report(&[1, 0, 0, 0], &payload(port_data(128, true))), None);
        }
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
