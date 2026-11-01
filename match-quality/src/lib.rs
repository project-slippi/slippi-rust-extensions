//! Match quality for Slippi Online.
//!
//! Decides when a match has degraded enough that it should be terminated. Instead of ping, it judges what the
//! players actually experience. Each interval is scored on three signals, and the worst one feeds a leaky debt
//! accumulator that ends the match once it fills up:
//!
//! - **Stall events.** The game halts whenever the opponent's latest input is more than `ROLLBACK_MAX_FRAMES`
//!   behind, because rolling back any further isn't possible. A run of halted frames is one stall event, and
//!   runs closer together than `STALL_MERGE_GAP_US` count as the same event, since a single spike often halts,
//!   releases a burst of inputs and halts again. Stalls under `MIN_STALL_PASSES` in total aren't events and add less
//!   debt, since the freeze is brief even though the rollback that follows is as deep as any. This catches
//!   connections that spike often even when they are fine in between.
//! - **Rollback depth.** How many frames the game is running ahead of the opponent's latest received input,
//!   which is how far back a rollback can reach. Delay frames are already accounted for, so this tracks how the
//!   match feels rather than raw latency. Averaged over an interval, it catches connections that are bad all the
//!   time.
//! - **Frame timing.** An interval that took longer than it should have in real time.
//!
//! Ping is never judged. It is only logged next to rollback depth so the depth tiers can be calibrated from real
//! games.
//!
//! All calls come from the CPU thread, but the state still lives behind a mutex like the other global
//! trackers.

use std::sync::{LazyLock, Mutex, MutexGuard};

use dolphin_integrations::Log;

/// The game halts once the opponent's latest input is further behind than this.
const ROLLBACK_MAX_FRAMES: i32 = 7;

const FRAME_TIME_US: u64 = 16683;

/// Nothing before this frame is judged, since the start of a game halts while both sides finish loading and
/// time sync lines them up. Stalls up to here are ignored, and the first debt interval starts here.
const WARMUP_FRAMES: i32 = 120;

/// The debt is updated every 150 frames (2.5 seconds) after the warmup.
const INTERVAL_FRAMES: i32 = 150;

/// Once a game has gone for a minute, every debt threshold is raised by this factor so it is a little harder for
/// debt to build up to termination.
const LATE_GAME_FRAME: i32 = 3600;
const LATE_GAME_MODIFIER: f64 = 1.15;

const TERMINATE_DEBT: i32 = 30;

/// Debt added for an interval in the mild, bad and severe tiers. A healthy interval pays one back.
const MILD_DEBT: i32 = 4;
const BAD_DEBT: i32 = 8;
const SEVERE_DEBT: i32 = 15;
const HEALTHY_DEBT: i32 = -1;

/// How far over the expected real time an interval ran (0.1 is 10% over) for each tier.
const OVERRUN_TIERS: [f64; 3] = [0.1, 0.5, 0.75];

/// Average rollback depth in frames for each tier. With a local delay of 2 these land around 105, 140 and 200 ms of
/// ping on a clean connection, and each extra delay frame adds about 33 ms.
const DEPTH_TIERS: [f64; 3] = [2.0, 3.0, 4.75];

/// Rollback depth is judged as if the local player has at least this much delay. Picking less accepts more rollbacks,
/// so it shouldn't lower the ping a match gets terminated at. Stalls aren't adjusted, since a halt is a real freeze
/// whichever delay caused it.
const MIN_JUDGED_DELAY: i32 = 2;

/// Stalled frames closer together than this belong to the same stall event.
const STALL_MERGE_GAP_US: u64 = 500_000;

/// A stall event only counts once it has been halted for this many passes in total, which is about 80 ms.
const MIN_STALL_PASSES: u32 = 5;

/// Debt for an interval holding at least one new stall event, however long it was. Against the leak this ends a
/// match at roughly 5 events in a minute or 10 in 3 minutes, and about 2.4 a minute can go on indefinitely. The
/// late game modifier doesn't apply.
const STALL_DEBT: i32 = 9;

/// Debt for an interval whose only stalls were under the minimum length. Against the leak, about 6 of these a minute
/// can go on indefinitely, while one every 5 seconds ends a match in about 75 seconds.
const SHORT_STALL_DEBT: i32 = 3;

/// Settings for a game, passed when it starts.
#[derive(Clone, Copy, Debug)]
pub struct GameConfig {
    /// Whether the match may be terminated. Only ranked matches are, but every game is tracked and logged.
    pub can_terminate: bool,
    /// The local player's delay frames.
    pub delay: u8,
}

/// Facts about one pass through the online frame handler. A halted frame is reported again on every pass
/// until it goes through.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub frame: i32,
    /// A monotonic clock reading.
    pub time_us: u64,
    /// The frame is halted because the opponent's inputs are too far behind.
    pub is_stall: bool,
    /// The newest frame we have the opponent's input for.
    pub latest_remote_frame: i32,
    /// The opponent's most recent ping measurement, or 0 if there hasn't been one yet. Only logged, never judged.
    pub ping_us: u64,
}

struct Game {
    config: GameConfig,
    /// The match has been judged too degraded to continue. This is only reported once.
    verdict_reached: bool,
    debt: i32,
    /// When the current debt interval started. Unset until the first interval boundary.
    interval_start_us: Option<u64>,
    /// The frame the last interval ended on. A frame held back for time sync is reported more than once, and
    /// must only end one interval.
    last_interval_frame: i32,
    depth_sum: u64,
    depth_samples: u64,
    ping_sum_us: u64,
    ping_samples: u64,
    last_stall_us: Option<u64>,
    /// Halted passes so far in the stall event in progress, or the most recent one.
    current_stall_passes: u32,
    /// Stall events that reached the minimum length in the current debt interval.
    interval_stall_events: u32,
    /// Halted passes in the current debt interval, including stalls too short to count as events.
    interval_stalled_passes: u32,
    /// Stall events so far this game.
    stall_events: u32,
}

impl Game {
    fn new(config: GameConfig) -> Self {
        Self {
            config,
            verdict_reached: false,
            debt: 0,
            interval_start_us: None,
            last_interval_frame: 0,
            depth_sum: 0,
            depth_samples: 0,
            ping_sum_us: 0,
            ping_samples: 0,
            last_stall_us: None,
            current_stall_passes: 0,
            interval_stall_events: 0,
            interval_stalled_passes: 0,
            stall_events: 0,
        }
    }

    /// Returns true once, when the match should be terminated.
    fn on_frame(&mut self, frame: Frame) -> bool {
        let delay_shortfall = (MIN_JUDGED_DELAY - i32::from(self.config.delay)).max(0);

        // A stalled frame is past the rollback limit by definition, so cap there in case the input queue is
        // empty and the latest frame reads as 0
        let depth = (frame.frame - frame.latest_remote_frame - delay_shortfall).clamp(0, ROLLBACK_MAX_FRAMES + 1);
        self.depth_sum += depth as u64;
        self.depth_samples += 1;

        if frame.ping_us > 0 {
            self.ping_sum_us += frame.ping_us;
            self.ping_samples += 1;
        }

        let reason = if frame.is_stall {
            self.on_stall(frame);
            None
        } else if frame.frame >= WARMUP_FRAMES
            && (frame.frame - WARMUP_FRAMES) % INTERVAL_FRAMES == 0
            && frame.frame != self.last_interval_frame
        {
            self.last_interval_frame = frame.frame;
            self.on_interval(frame)
        } else {
            None
        };

        let Some(reason) = reason else {
            return false;
        };

        // A match that can't be terminated keeps being tracked, so its log covers the whole game
        if self.verdict_reached {
            return false;
        }

        self.verdict_reached = true;

        if !self.config.can_terminate {
            tracing::info!(
                target: Log::SlippiOnline,
                frame = frame.frame,
                "[MatchQuality] A ranked match would have been terminated here: {reason}"
            );
            return false;
        }

        tracing::warn!(target: Log::SlippiOnline, frame = frame.frame, "[MatchQuality] Terminating match: {reason}");
        true
    }

    fn on_stall(&mut self, frame: Frame) {
        if frame.frame <= WARMUP_FRAMES {
            return;
        }

        self.interval_stalled_passes += 1;

        let is_new_event = self
            .last_stall_us
            .is_none_or(|last| frame.time_us.saturating_sub(last) > STALL_MERGE_GAP_US);
        self.last_stall_us = Some(frame.time_us);

        if is_new_event {
            self.current_stall_passes = 0;
        }
        self.current_stall_passes += 1;

        // An event counts once, on the pass that reaches the minimum length
        if self.current_stall_passes != MIN_STALL_PASSES {
            return;
        }

        self.interval_stall_events += 1;
        self.stall_events += 1;

        tracing::info!(
            target: Log::SlippiOnline,
            frame = frame.frame,
            latest_remote_frame = frame.latest_remote_frame,
            "[MatchQuality] Stall event. {} this game",
            self.stall_events
        );
    }

    fn on_interval(&mut self, frame: Frame) -> Option<String> {
        let depth_sum = std::mem::take(&mut self.depth_sum);
        let depth_samples = std::mem::take(&mut self.depth_samples);
        let ping_sum_us = std::mem::take(&mut self.ping_sum_us);
        let ping_samples = std::mem::take(&mut self.ping_samples);
        let stall_events = std::mem::take(&mut self.interval_stall_events);
        let stalled_passes = std::mem::take(&mut self.interval_stalled_passes);

        // The first boundary only starts the clock
        let start_us = self.interval_start_us.replace(frame.time_us)?;

        let expected_us = FRAME_TIME_US * INTERVAL_FRAMES as u64;
        let ratio = frame.time_us.saturating_sub(start_us) as f64 / expected_us as f64;
        let avg_depth = depth_sum as f64 / depth_samples.max(1) as f64;
        let avg_ping_ms = ping_sum_us as f64 / ping_samples.max(1) as f64 / 1000.0;

        let modifier = if frame.frame > LATE_GAME_FRAME {
            LATE_GAME_MODIFIER
        } else {
            1.0
        };
        let speed_debt = tier_debt(ratio - 1.0, OVERRUN_TIERS, modifier);
        let depth_debt = tier_debt(avg_depth, DEPTH_TIERS, modifier);
        let stall_debt = if stall_events > 0 {
            STALL_DEBT
        } else if stalled_passes > 0 {
            SHORT_STALL_DEBT
        } else {
            HEALTHY_DEBT
        };

        // Take the worst of the three rather than summing: a bad connection stalls, deepens rollbacks and stretches
        // the interval (halting to wait for inputs) all at once, so summing would count one cause several times
        let change = speed_debt.max(depth_debt).max(stall_debt);
        self.debt = (self.debt + change).max(0);

        let delay = self.config.delay;
        tracing::info!(
            target: Log::SlippiOnline,
            frame = frame.frame,
            "[MatchQuality] Debt {change:+} (speed: {speed_debt}, depth: {depth_debt}, stall: {stall_debt}, ratio: \
             {ratio:.2}, avg depth: {avg_depth:.2}, delay: {delay}, avg ping: {avg_ping_ms:.1}ms, stall events: \
             {stall_events}, stalled passes: {stalled_passes}). Now {}/{TERMINATE_DEBT}",
            self.debt
        );

        (self.debt >= TERMINATE_DEBT).then(|| {
            format!(
                "poor performance debt {}/{TERMINATE_DEBT} (ratio: {ratio:.2}, avg depth: {avg_depth:.2}, avg ping: \
                 {avg_ping_ms:.1}ms, stall events this game: {})",
                self.debt, self.stall_events
            )
        })
    }
}

/// The debt for a value measured against mild, bad and severe thresholds, each scaled by `modifier`.
fn tier_debt(value: f64, tiers: [f64; 3], modifier: f64) -> i32 {
    let [mild, bad, severe] = tiers.map(|t| t * modifier);
    if value >= severe {
        SEVERE_DEBT
    } else if value >= bad {
        BAD_DEBT
    } else if value >= mild {
        MILD_DEBT
    } else {
        HEALTHY_DEBT
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

/// Stops tracking the current game.
pub fn end_game() {
    *lock() = None;
}

/// Called on the CPU thread for every pass through the online frame handler, halted or not. Returns true once,
/// when the match should be terminated.
pub fn on_frame(frame: Frame) -> bool {
    lock().as_mut().is_some_and(|game| game.on_frame(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives a game one pass at a time, a frame time apart.
    struct Sim {
        game: Game,
        frame: i32,
        time_us: u64,
        /// How far behind the opponent's latest input is when the game isn't stalled.
        depth: i32,
        terminated: bool,
    }

    impl Sim {
        fn new(can_terminate: bool) -> Self {
            Self {
                game: Game::new(GameConfig { can_terminate, delay: 2 }),
                frame: 1,
                time_us: 1_000_000,
                depth: 1,
                terminated: false,
            }
        }

        fn pass(&mut self, is_stall: bool) {
            let depth = if is_stall { ROLLBACK_MAX_FRAMES + 1 } else { self.depth };
            let terminate = self.game.on_frame(Frame {
                frame: self.frame,
                time_us: self.time_us,
                is_stall,
                latest_remote_frame: self.frame - depth,
                ping_us: 0,
            });

            assert!(!(terminate && self.terminated), "termination reported twice");
            self.terminated |= terminate;
            self.time_us += FRAME_TIME_US;
            if !is_stall {
                self.frame += 1;
            }
        }

        fn run(&mut self, frames: i32) {
            for _ in 0..frames {
                self.pass(false);
            }
        }

        fn run_seconds(&mut self, seconds: i32) {
            self.run(seconds * 60);
        }

        fn stall(&mut self, passes: u32) {
            for _ in 0..passes {
                self.pass(true);
            }
        }
    }

    fn warmed_up(can_terminate: bool) -> Sim {
        let mut sim = Sim::new(can_terminate);
        sim.run(WARMUP_FRAMES);
        sim
    }

    #[test]
    fn healthy_match_is_not_terminated() {
        let mut sim = warmed_up(true);
        sim.run_seconds(480);
        assert!(!sim.terminated);
        assert_eq!(sim.game.debt, 0);
    }

    /// Runs a stall followed by healthy play until the match terminates, returning how many stalls that took.
    fn stalls_to_terminate(gap_seconds: i32, max_stalls: i32) -> Option<i32> {
        let mut sim = warmed_up(true);
        for stalls in 1..=max_stalls {
            sim.stall(10);
            sim.run_seconds(gap_seconds);
            if sim.terminated {
                return Some(stalls);
            }
        }
        None
    }

    #[test]
    fn five_stall_events_in_a_minute_terminate() {
        assert_eq!(stalls_to_terminate(12, 20), Some(5));
    }

    #[test]
    fn slower_stall_events_take_more_of_them() {
        // Roughly 10 in 3 minutes
        let stalls = stalls_to_terminate(18, 20).unwrap();
        assert!((8..=10).contains(&stalls), "took {stalls} stalls");
    }

    #[test]
    fn stall_events_the_leak_keeps_up_with_survive() {
        assert_eq!(stalls_to_terminate(26, 20), None);
    }

    #[test]
    fn close_stalls_are_one_event() {
        let mut sim = warmed_up(true);
        // Halting and releasing every few frames during one spike
        for _ in 0..20 {
            sim.stall(2);
            sim.run(5);
        }
        assert_eq!(sim.game.stall_events, 1);
        assert_eq!(sim.game.interval_stalled_passes, 40);
        assert!(!sim.terminated);
    }

    #[test]
    fn stall_events_in_one_interval_add_debt_once() {
        let mut sim = warmed_up(true);
        // More than the merge gap apart, so these are separate events
        for _ in 0..3 {
            sim.stall(MIN_STALL_PASSES);
            sim.run(40);
        }
        assert_eq!(sim.game.stall_events, 3);

        sim.run(INTERVAL_FRAMES - 3 * 40);
        assert_eq!(sim.game.debt, STALL_DEBT);
    }

    #[test]
    fn stalls_under_the_minimum_length_add_less_debt() {
        let mut sim = warmed_up(true);
        // More than the merge gap apart, so these don't add up to one event
        for _ in 0..3 {
            sim.stall(MIN_STALL_PASSES - 1);
            sim.run(40);
        }
        sim.run(INTERVAL_FRAMES - 3 * 40);
        assert_eq!(sim.game.stall_events, 0);
        assert_eq!(sim.game.debt, SHORT_STALL_DEBT);
    }

    #[test]
    fn only_frequent_short_stalls_terminate() {
        let terminates = |gap_seconds| {
            let mut sim = warmed_up(true);
            for _ in 0..60 {
                sim.stall(MIN_STALL_PASSES - 1);
                sim.run_seconds(gap_seconds);
            }
            sim.terminated
        };
        assert!(terminates(5));
        assert!(!terminates(10));
    }

    #[test]
    fn stall_debt_is_the_worst_signal_not_a_sum() {
        // A 60 pass stall also scores bad depth debt, and a 120 pass stall scores severe speed debt
        for (passes, debt) in [(MIN_STALL_PASSES, STALL_DEBT), (60, STALL_DEBT), (120, SEVERE_DEBT)] {
            let mut sim = warmed_up(true);
            sim.stall(passes);
            sim.run(INTERVAL_FRAMES);
            assert_eq!(sim.game.debt, debt, "{passes} passes");
        }
    }

    #[test]
    fn halts_during_warmup_leave_no_debt() {
        let mut sim = Sim::new(true);
        sim.run(60);
        // A second of halting to wait for the opponent to finish loading. If these samples carried into the first
        // interval they would push its average depth into the mild tier
        sim.stall(60);
        sim.run(WARMUP_FRAMES - 60);
        sim.run(4 * INTERVAL_FRAMES);
        assert_eq!(sim.game.debt, 0);
        assert_eq!(sim.game.stall_events, 0);
    }

    #[test]
    fn constant_bad_depth_terminates_after_four_intervals() {
        let mut sim = Sim::new(true);
        sim.depth = 3;
        // The first boundary at the end of the warmup only starts the clock, and four bad intervals reach 32
        sim.run(WARMUP_FRAMES + 3 * INTERVAL_FRAMES);
        assert!(!sim.terminated);

        sim.run(INTERVAL_FRAMES);
        assert!(sim.terminated);
    }

    #[test]
    fn mild_depth_needs_the_late_game_bar() {
        let mut sim = Sim::new(true);
        sim.depth = 2;
        sim.run(WARMUP_FRAMES + 4 * INTERVAL_FRAMES);
        assert_eq!(sim.game.debt, 16);

        // Past a minute, 2 frames is under the raised mild threshold of 2.3
        let mut sim = Sim::new(true);
        sim.depth = 2;
        sim.frame = LATE_GAME_FRAME;
        sim.run(10 * INTERVAL_FRAMES);
        assert_eq!(sim.game.debt, 0);
        assert!(!sim.terminated);
    }

    #[test]
    fn delay_below_the_minimum_is_judged_as_the_minimum() {
        // 3 frames at delay 1 is the ping that reads as 2 frames at delay 2, so it scores mild instead of bad
        let mut sim = Sim::new(true);
        sim.game.config.delay = 1;
        sim.depth = 3;
        sim.run(WARMUP_FRAMES + INTERVAL_FRAMES);
        assert_eq!(sim.game.debt, MILD_DEBT);
    }

    #[test]
    fn delay_above_the_minimum_is_not_adjusted() {
        let mut sim = Sim::new(true);
        sim.game.config.delay = 4;
        sim.depth = 3;
        sim.run(WARMUP_FRAMES + INTERVAL_FRAMES);
        assert_eq!(sim.game.debt, BAD_DEBT);
    }

    #[test]
    fn stalls_on_delay_one_still_add_debt() {
        let mut sim = warmed_up(true);
        sim.game.config.delay = 1;
        sim.stall(MIN_STALL_PASSES);
        sim.run(INTERVAL_FRAMES);
        assert_eq!(sim.game.debt, STALL_DEBT);
    }

    #[test]
    fn slow_intervals_add_debt() {
        let mut sim = Sim::new(true);
        sim.run(WARMUP_FRAMES);

        // Each interval takes twice as long as it should
        for _ in 0..2 * INTERVAL_FRAMES {
            sim.time_us += FRAME_TIME_US;
            sim.pass(false);
        }
        assert_eq!(sim.game.debt, 2 * SEVERE_DEBT);
        assert!(sim.terminated);
    }

    #[test]
    fn first_boundary_held_by_time_sync_adds_no_debt() {
        let mut sim = Sim::new(true);
        sim.depth = 3;
        sim.run(WARMUP_FRAMES - 1);

        // Time sync only holds frames back this early in a game, which makes this the one boundary it can repeat
        for _ in 0..5 {
            sim.game.on_frame(Frame {
                frame: sim.frame,
                time_us: sim.time_us,
                is_stall: false,
                latest_remote_frame: sim.frame - sim.depth,
                ping_us: 0,
            });
            sim.time_us += FRAME_TIME_US;
        }
        assert_eq!(sim.game.debt, 0);
    }

    #[test]
    fn unranked_matches_are_only_logged() {
        let mut sim = warmed_up(false);
        for _ in 0..6 {
            sim.stall(10);
            sim.run_seconds(5);
        }
        assert!(!sim.terminated);
        assert!(sim.game.verdict_reached);

        // The verdict comes before the last stalls, and they are still tracked
        assert_eq!(sim.game.stall_events, 6);
    }
}
