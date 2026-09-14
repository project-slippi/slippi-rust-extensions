//! The background thread that owns the Discord IPC connection and turns
//! parsed replay events and pushed matchmaking state into presence updates.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use discord_rich_presence::activity::{Activity, Assets, Button, Party, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use dolphin_integrations::Log;
use slippi_user::{RankFetchStatus, UserManager};

use crate::Message;
use crate::content::{GameState, PresenceContent, SetScore, game_details, game_state_line, rank_asset_key, rank_name, unix_time};
use crate::melee;
use crate::menu::{MatchmakingState, ProcessState};
use crate::parser::{EventParser, SlpEvent};
use crate::scene::SceneState;

/// The Discord application that hosts the presence image assets.
const DISCORD_CLIENT_ID: &str = "1096595344600604772";

/// Base URL for the "Get Slippi" and profile buttons shown to others.
const SLIPPI_URL: &str = "https://slippi.gg/";

/// How often the thread wakes up to flush throttled updates and retry the
/// Discord connection when no data is arriving.
const TICK_INTERVAL: Duration = Duration::from_millis(500);

/// Discord rate-limits activity updates to roughly one every four seconds.
const MIN_UPDATE_INTERVAL: Duration = Duration::from_secs(4);

/// How long to wait between attempts to (re)connect to a Discord client.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);

/// All presence state lives here, owned by the presence thread and mutated only
/// in response to a `Message`. Nothing on the C++/EXI side ever reads it back,
/// so unlike `GameReporterQueue` or `UserManager` there's no `Arc<Mutex<>>` to
/// share; the channel is the only way in, which keeps the locking story empty.
pub(crate) struct PresenceRunner {
    client: Option<DiscordIpcClient>,
    last_connect_attempt: Option<Instant>,
    last_update: Option<Instant>,
    pending_update: bool,
    activity_visible: bool,
    game: Option<GameState>,
    set_score: SetScore,
    matchmaking: MatchmakingState,
    scene: SceneState,
    user_manager: Option<UserManager>,
    show_rank: bool,
}

/// Thread entry point: drains the message channel, feeds the parser, and
/// pushes presence updates until shutdown.
pub(crate) fn run(receiver: Receiver<Message>, user_manager: Option<UserManager>) -> crate::Result<()> {
    let mut parser = EventParser::new();

    let mut runner = PresenceRunner::new(user_manager);

    loop {
        // Wake periodically to flush rate-limited updates.
        let message = match receiver.recv_timeout(TICK_INTERVAL) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,

            // Normal shutdown goes through Message::Shutdown, so a bare disconnect is unexpected.
            Err(error @ RecvTimeoutError::Disconnected) => return Err(error.into()),
        };

        match message {
            Some(Message::ReplayData(data)) => {
                parser.push(&data, |event| runner.handle_event(event));
            },

            Some(Message::Matchmaking {
                process_state,
                online_mode,
                opponent_name,
                opponent_rank,
            }) => {
                let state = MatchmakingState::from_ffi(process_state, online_mode, opponent_name, opponent_rank);
                runner.handle_matchmaking(state);
            },

            Some(Message::Scene {
                major,
                minor,
                char_ids,
                local_port,
                stage_id,
            }) => {
                let state = SceneState::from_ffi(major, minor, char_ids, local_port, stage_id);
                runner.handle_scene(state);
            },

            Some(Message::ShowRank(show_rank)) => {
                if runner.show_rank != show_rank {
                    runner.show_rank = show_rank;
                    runner.pending_update = true;
                }
            },

            Some(Message::Shutdown) => break,
            None => {},
        }

        runner.flush();
    }

    runner.clear();
    tracing::info!(target: Log::DiscordRpc, "Discord presence thread shutting down");
    Ok(())
}

impl PresenceRunner {
    /// Creates a runner with no Discord connection yet and default state.
    fn new(user_manager: Option<UserManager>) -> Self {
        Self {
            client: None,
            last_connect_attempt: None,
            last_update: None,
            // Start dirty so the baseline "in menus" presence shows as soon as
            // we connect, before any game or matchmaking frame arrives.
            pending_update: true,
            activity_visible: false,
            game: None,
            set_score: SetScore::default(),
            matchmaking: MatchmakingState::default(),
            scene: SceneState::default(),
            user_manager,
            // Default on; Dolphin overrides via set_show_rank.
            show_rank: true,
        }
    }

    /// Folds a parsed replay event into the current game state, flagging a
    /// presence refresh when something player-visible changed.
    fn handle_event(&mut self, event: SlpEvent) {
        match event {
            SlpEvent::GameStart(info) => {
                if self.set_score.match_id != info.match_id {
                    self.set_score = SetScore {
                        match_id: info.match_id.clone(),
                        wins: [0; 4],
                    };
                }

                tracing::info!(
                    target: Log::DiscordRpc,
                    stage = ?melee::stage_name(info.stage_id),
                    match_id = %info.match_id,
                    game_number = info.game_number,
                    "Game started"
                );

                let mut stocks = [0; 4];

                for player in &info.players {
                    stocks[player.port as usize] = player.starting_stocks;
                }

                self.game = Some(GameState {
                    info,
                    stocks,
                    started_at: unix_time(),
                    ended: false,
                });

                self.pending_update = true;
            },

            SlpEvent::PostFrame { player, stocks, .. } => {
                let Some(game) = self.game.as_mut() else { return };

                if game.ended || player as usize >= 4 {
                    return;
                }

                if game.stocks[player as usize] != stocks {
                    game.stocks[player as usize] = stocks;
                    self.pending_update = true;
                }
            },

            SlpEvent::GameEnd { placements, .. } => {
                let Some(game) = self.game.as_mut() else { return };

                game.ended = true;

                if let Some(winner_port) = placements.iter().position(|&p| p == 0) {
                    if winner_port < 4 {
                        self.set_score.wins[winner_port] += 1;
                    }
                }

                tracing::info!(
                    target: Log::DiscordRpc,
                    wins = ?self.set_score.wins,
                    "Game ended"
                );

                self.pending_update = true;
            },
        }
    }

    /// Folds a pushed matchmaking snapshot into presence state. A live game
    /// keeps ownership of presence until it ends.
    fn handle_matchmaking(&mut self, state: MatchmakingState) {
        if state == self.matchmaking {
            return;
        }

        let game_is_live = self.game.as_ref().is_some_and(|game| !game.ended);

        // A live game owns presence; just record the new state for later.
        if game_is_live && state.process == ProcessState::ConnectionSuccess {
            self.matchmaking = state;
            return;
        }

        // Back to matchmaking with no live game: drop any finished game so its
        // result stops showing.
        if !game_is_live && state.process != ProcessState::ConnectionSuccess {
            self.game = None;
        }

        tracing::debug!(target: Log::DiscordRpc, ?state, "Matchmaking state changed");
        self.matchmaking = state;
        self.pending_update = true;
    }

    /// Folds a pushed scene snapshot into presence state. A live game keeps
    /// ownership of presence; returning to the character-select screen clears a
    /// finished game so its result stops showing.
    fn handle_scene(&mut self, state: SceneState) {
        if state == self.scene {
            return;
        }

        // Log only on a real scene change, not on every cursor move, so the
        // raw major/minor is visible while testing without flooding the log.
        if (state.major, state.minor) != (self.scene.major, self.scene.minor) {
            tracing::info!(target: Log::DiscordRpc, major = state.major, minor = state.minor, "Scene changed");
        }

        // Back on the character-select screen with a finished game still around:
        // drop it so the offline result stops showing.
        if state.is_css() && self.game.as_ref().is_some_and(|game| game.ended) {
            self.game = None;
        }

        self.scene = state;
        self.pending_update = true;
    }

    /// Pushes the current state to Discord, respecting the rate limit.
    fn flush(&mut self) {
        if !self.pending_update {
            return;
        }

        if let Some(last) = self.last_update {
            if last.elapsed() < MIN_UPDATE_INTERVAL {
                return;
            }
        }

        if !self.ensure_connected() {
            return;
        }

        let Some(content) = self.content() else {
            self.clear();
            self.pending_update = false;
            return;
        };

        let mut assets = Assets::new()
            .large_image(&content.large_image)
            .large_text(&content.large_text);

        if let Some(image) = &content.small_image {
            assets = assets.small_image(image);
        }

        if let Some(text) = &content.small_text {
            assets = assets.small_text(text);
        }

        // Owned so the Button borrows outlive set_activity.
        let button_data = self.buttons();
        let buttons = button_data
            .iter()
            .map(|(label, url)| Button::new(label.as_str(), url.as_str()))
            .collect();

        let mut activity = Activity::new()
            .details(&content.details)
            .state(&content.state)
            .assets(assets)
            .buttons(buttons);

        if let Some(start) = content.start_timestamp {
            activity = activity.timestamps(Timestamps::new().start(start));
        }

        if let Some((current, max)) = content.party {
            activity = activity.party(Party::new().size([current as i32, max as i32]));
        }

        let Some(client) = self.client.as_mut() else {
            return;
        };

        match client.set_activity(activity) {
            Ok(_) => {
                self.pending_update = false;
                self.activity_visible = true;
                self.last_update = Some(Instant::now());
                tracing::info!(
                    target: Log::DiscordRpc,
                    details = %content.details,
                    state = %content.state,
                    "Updated Discord presence"
                );
            },

            Err(error) => {
                tracing::warn!(target: Log::DiscordRpc, ?error, "Failed to update Discord presence, dropping connection");
                self.client = None;
            },
        }
    }

    /// Decides what presence to show right now, or `None` to show nothing.
    fn content(&self) -> Option<PresenceContent> {
        // A live or just-finished game always wins.
        if let Some(game) = &self.game {
            return Some(self.game_content(game));
        }

        // The online matchmaking flow owns presence whenever it's active.
        if !matches!(self.matchmaking.process, ProcessState::Idle | ProcessState::Error) {
            return Some(self.matchmaking_content());
        }

        // Offline scene presence: character select, training, the 1P modes,
        // offline Vs before the game starts producing replay data.
        if let Some(content) = self.scene_content() {
            return Some(content);
        }

        // Default: the online idle baseline ("In menus").
        Some(self.matchmaking_content())
    }

    /// Presence for the online menu and matchmaking flow, keyed off the pushed
    /// `MatchmakingState`.
    fn matchmaking_content(&self) -> PresenceContent {
        let mm = &self.matchmaking;
        match mm.process {
            ProcessState::Idle | ProcessState::Error => {
                let (large_image, large_text) = self.player_badge();
                PresenceContent {
                    details: "Slippi Online".to_string(),
                    state: "In menus".to_string(),
                    large_image,
                    large_text,
                    ..Default::default()
                }
            },

            ProcessState::Initializing | ProcessState::Matchmaking => {
                let (large_image, large_text) = self.player_badge();
                let details = match mm.mode {
                    Some(mode) => format!("In queue - {}", mode.name()),
                    None => "In queue".to_string(),
                };
                PresenceContent {
                    details,
                    state: "Searching for an opponent".to_string(),
                    large_image,
                    large_text,
                    party: Some((1, 2)),
                    ..Default::default()
                }
            },

            ProcessState::OpponentConnecting => {
                let (large_image, large_text) = self.opponent_badge();
                PresenceContent {
                    details: "Opponent found".to_string(),
                    state: match &mm.opponent_name {
                        Some(name) => format!("vs {name}"),
                        None => "Connecting...".to_string(),
                    },
                    large_image,
                    large_text,
                    party: Some((2, 2)),
                    ..Default::default()
                }
            },

            ProcessState::ConnectionSuccess => {
                let (large_image, large_text) = self.player_badge();
                PresenceContent {
                    details: "Starting match...".to_string(),
                    state: match &mm.opponent_name {
                        Some(name) => format!("vs {name}"),
                        None => "Starting...".to_string(),
                    },
                    large_image,
                    large_text,
                    party: Some((2, 2)),
                    ..Default::default()
                }
            },
        }
    }

    /// Presence for the offline scenes that never reach the replay stream: the
    /// character-select screen (showing the local player's hovered character)
    /// and the recognized offline modes. `None` when the current scene isn't
    /// one we render, so the caller falls back to the menu baseline.
    fn scene_content(&self) -> Option<PresenceContent> {
        let scene = &self.scene;

        if scene.is_css() {
            let local_char = scene.local_char_id();

            let (small_image, small_text) = match local_char {
                Some(id) => (Some(melee::character_asset(id)), melee::character_name(id).map(String::from)),
                None => (None, None),
            };

            let (large_image, large_text) = self.player_badge();

            return Some(PresenceContent {
                details: "Choosing characters".to_string(),
                state: match local_char.and_then(melee::character_name) {
                    Some(name) => name.to_string(),
                    None => "Character select".to_string(),
                },
                large_image,
                large_text,
                small_image,
                small_text,
                ..Default::default()
            });
        }

        let mode = scene.mode_label()?;

        // Stage-select screen: show that a stage is being picked.
        if scene.is_sss() {
            let (large_image, large_text) = self.player_badge();
            return Some(PresenceContent {
                details: mode.to_string(),
                state: "Choosing a stage".to_string(),
                large_image,
                large_text,
                ..Default::default()
            });
        }

        // A loaded stage (e.g. training on a stage) is featured as the large
        // image, the same way an in-game match is.
        if let Some(stage_id) = scene.stage() {
            if let Some(name) = melee::stage_name_internal(stage_id as u16) {
                return Some(PresenceContent {
                    details: mode.to_string(),
                    state: name.to_string(),
                    large_image: melee::stage_asset_internal(stage_id as u16),
                    large_text: name.to_string(),
                    ..Default::default()
                });
            }
        }

        let (large_image, large_text) = self.player_badge();

        Some(PresenceContent {
            details: mode.to_string(),
            state: "Offline".to_string(),
            large_image,
            large_text,
            ..Default::default()
        })
    }

    /// Large image + hover text for the opponent-found state: the opponent's
    /// rank badge when known and rank display is on, otherwise the Slippi logo.
    fn opponent_badge(&self) -> (String, String) {
        if self.show_rank {
            if let Some(rank) = self.matchmaking.opponent_rank.filter(|&r| r > 0) {
                if let Some(name) = rank_name(rank) {
                    return (rank_asset_key(name), name.to_string());
                }
            }
        }

        self.player_badge()
    }

    /// The player's rank badge as (asset key, label) once a real rank has been
    /// fetched, or `None` when the player has hidden their rank in-game.
    fn rank_badge(&self) -> Option<(String, String)> {
        if !self.show_rank {
            return None;
        }

        let user_manager = self.user_manager.as_ref()?;
        let (info, status) = user_manager.current_rank_and_status();

        if !matches!(status, RankFetchStatus::Fetched) || info.rank == 0 {
            return None;
        }

        let name = rank_name(info.rank)?;
        let asset = rank_asset_key(name);
        let text = format!("{name} · {}", info.rating_ordinal.round() as i32);

        Some((asset, text))
    }

    /// Large image + hover text representing the local player in any non-game
    /// state: their rank badge once fetched, otherwise the generic Slippi logo.
    fn player_badge(&self) -> (String, String) {
        self.rank_badge()
            .unwrap_or_else(|| ("slippi".to_string(), "Slippi Online".to_string()))
    }

    /// Buttons shown to other viewers: a Get Slippi link plus the player's
    /// slippi.gg profile when their connect code is known.
    fn buttons(&self) -> Vec<(String, String)> {
        let mut buttons = vec![("Get Slippi".to_string(), SLIPPI_URL.to_string())];

        if let Some(url) = self.profile_url() {
            buttons.push(("View Slippi Profile".to_string(), url));
        }

        buttons
    }

    /// The slippi.gg profile URL for the logged-in player, e.g. `SWOO#0` ->
    /// `https://slippi.gg/user/swoo-0`, if their connect code is known.
    fn profile_url(&self) -> Option<String> {
        let code = self.user_manager.as_ref()?.get(|user| user.connect_code.clone());

        code.contains('#')
            .then(|| format!("{SLIPPI_URL}user/{}", code.to_lowercase().replace('#', "-")))
    }

    /// Presence for an in-progress or just-finished game: stage as the large
    /// image, the local player's character as the small image, and the live
    /// stock line / set score as details.
    fn game_content(&self, game: &GameState) -> PresenceContent {
        let stage_asset = melee::stage_asset(game.info.stage_id);
        let stage_text = melee::stage_name(game.info.stage_id).unwrap_or("Unknown stage").to_string();

        let (small_image, small_text) = match game.info.players.first() {
            Some(player) => (
                Some(melee::character_asset(player.character_id)),
                melee::character_name(player.character_id).map(String::from),
            ),
            None => (None, None),
        };

        let players = game.info.players.len() as u32;

        PresenceContent {
            details: game_details(game, &self.set_score),
            state: game_state_line(game),
            large_image: stage_asset,
            large_text: stage_text,
            small_image,
            small_text,
            start_timestamp: Some(game.started_at),
            party: (players > 0).then_some((players, players)),
        }
    }

    /// Connects to a running Discord client if not already connected, with a
    /// reconnect cooldown.
    fn ensure_connected(&mut self) -> bool {
        if self.client.is_some() {
            return true;
        }

        if let Some(last) = self.last_connect_attempt {
            if last.elapsed() < RECONNECT_INTERVAL {
                return false;
            }
        }

        self.last_connect_attempt = Some(Instant::now());

        let mut client = DiscordIpcClient::new(DISCORD_CLIENT_ID);

        match client.connect() {
            Ok(_) => {
                tracing::info!(target: Log::DiscordRpc, "Connected to Discord");
                self.client = Some(client);
                true
            },

            Err(error) => {
                tracing::warn!(target: Log::DiscordRpc, ?error, "Could not connect to Discord (is it running?)");
                false
            },
        }
    }

    /// Clears the Discord activity if one is currently shown.
    fn clear(&mut self) {
        if !self.activity_visible {
            return;
        }

        if let Some(client) = self.client.as_mut() {
            let _ = client.clear_activity();
            self.activity_visible = false;
        }
    }
}

impl Drop for PresenceRunner {
    fn drop(&mut self) {
        if let Some(client) = self.client.as_mut() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::OnlineMode;

    fn test_runner() -> PresenceRunner {
        PresenceRunner::new(None)
    }

    fn state(process: ProcessState) -> MatchmakingState {
        MatchmakingState {
            process,
            ..Default::default()
        }
    }

    #[test]
    fn matchmaking_change_flags_an_update() {
        let mut runner = test_runner();
        runner.pending_update = false;
        runner.handle_matchmaking(state(ProcessState::Matchmaking));
        assert_eq!(runner.matchmaking.process, ProcessState::Matchmaking);
        assert!(runner.pending_update);
    }

    #[test]
    fn identical_matchmaking_state_is_a_no_op() {
        let mut runner = test_runner();
        runner.matchmaking = state(ProcessState::Matchmaking);
        runner.pending_update = false;
        runner.handle_matchmaking(state(ProcessState::Matchmaking));
        assert!(!runner.pending_update);
    }

    #[test]
    fn starts_dirty_so_the_baseline_presence_shows() {
        assert!(test_runner().pending_update);
    }

    #[test]
    fn a_live_game_survives_a_connection_success_frame() {
        let mut runner = test_runner();
        runner.game = Some(GameState {
            ended: false,
            ..Default::default()
        });
        runner.handle_matchmaking(state(ProcessState::ConnectionSuccess));
        assert!(runner.game.is_some());
    }

    #[test]
    fn a_finished_game_clears_when_back_in_menus() {
        let mut runner = test_runner();
        runner.matchmaking = state(ProcessState::ConnectionSuccess);
        runner.game = Some(GameState {
            ended: true,
            ..Default::default()
        });
        runner.handle_matchmaking(state(ProcessState::Idle));
        assert!(runner.game.is_none());
    }

    #[test]
    fn idle_renders_in_menus() {
        let content = test_runner().content().unwrap();
        assert_eq!(content.details, "Slippi Online");
        assert_eq!(content.state, "In menus");
    }

    #[test]
    fn queueing_renders_mode_and_party() {
        let mut runner = test_runner();
        runner.matchmaking = MatchmakingState {
            process: ProcessState::Matchmaking,
            mode: Some(OnlineMode::Ranked),
            ..Default::default()
        };
        let content = runner.content().unwrap();
        assert_eq!(content.details, "In queue - Ranked");
        assert_eq!(content.party, Some((1, 2)));
    }

    #[test]
    fn opponent_connecting_renders_their_name() {
        let mut runner = test_runner();
        runner.matchmaking = MatchmakingState {
            process: ProcessState::OpponentConnecting,
            opponent_name: Some("Mango".to_string()),
            ..Default::default()
        };
        let content = runner.content().unwrap();
        assert_eq!(content.details, "Opponent found");
        assert_eq!(content.state, "vs Mango");
    }

    /// A CSS scene (Versus major, CSS minor) with Falco hovered on the local
    /// port renders "Choosing characters" + the character icon.
    fn falco_css_scene() -> SceneState {
        SceneState::from_ffi(0x02, 0x00, [0xFF, 0x14, 0xFF, 0xFF], 1, 0)
    }

    #[test]
    fn css_scene_renders_choosing_characters_with_hovered_char() {
        let mut runner = test_runner();
        runner.scene = falco_css_scene();
        let content = runner.content().unwrap();
        assert_eq!(content.details, "Choosing characters");
        assert_eq!(content.state, "Falco");
        assert_eq!(content.small_image.as_deref(), Some("char20"));
    }

    #[test]
    fn a_live_game_outranks_a_css_scene() {
        let mut runner = test_runner();
        runner.scene = falco_css_scene();
        runner.game = Some(GameState {
            ended: false,
            ..Default::default()
        });
        // game_details falls back to "Online" for an empty match id.
        assert_eq!(runner.content().unwrap().details, "Online");
    }

    #[test]
    fn active_matchmaking_outranks_a_css_scene() {
        let mut runner = test_runner();
        runner.scene = falco_css_scene();
        runner.matchmaking = state(ProcessState::Matchmaking);
        assert_eq!(runner.content().unwrap().details, "In queue");
    }

    #[test]
    fn idle_with_a_css_scene_shows_the_scene() {
        let mut runner = test_runner();
        runner.scene = falco_css_scene();
        // Idle matchmaking, no game: the scene wins over the "In menus" baseline.
        assert_eq!(runner.content().unwrap().details, "Choosing characters");
    }

    #[test]
    fn returning_to_css_clears_a_finished_game() {
        let mut runner = test_runner();
        runner.game = Some(GameState {
            ended: true,
            ..Default::default()
        });
        runner.handle_scene(falco_css_scene());
        assert!(runner.game.is_none());
    }
}
