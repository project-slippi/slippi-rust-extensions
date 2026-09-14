//! This module houses the `SlippiEXIDevice`, which is in effect a "shadow subclass" of the C++
//! Slippi EXI device.
//!
//! What this means is that the Slippi EXI Device (C++) holds a pointer to the Rust
//! `SlippiEXIDevice` and forwards calls over the C FFI. This has a fairly clean mapping to "when
//! Slippi stuff is happening" and enables us to let the Rust side live in its own world.

use dolphin_integrations::Log;
use slippi_discord_rpc::DiscordHandler;
use slippi_game_reporter::GameReporter;
use slippi_gg_api::APIClient;
use slippi_jukebox::Jukebox;
use slippi_user::UserManager;

mod config;
pub use config::{Config, FilePathsConfig, SCMConfig};

/// An EXI Device subclass specific to managing and interacting with the game itself.
#[derive(Debug)]
pub struct SlippiEXIDevice {
    config: Config,
    pub game_reporter: GameReporter,
    pub user_manager: UserManager,
    pub jukebox: Option<Jukebox>,
    pub discord_rpc: Option<DiscordHandler>,
}

pub enum JukeboxConfiguration {
    Start {
        initial_dolphin_system_volume: u8,
        initial_dolphin_music_volume: u8,
    },
    Stop,
}

pub enum DiscordRpcConfiguration {
    Start { show_rank: bool },
    Stop,
}

/// `None` for an empty string.
fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

impl SlippiEXIDevice {
    /// Creates and returns a new `SlippiEXIDevice` with default values.
    ///
    /// At the moment you should never need to call this yourself.
    pub fn new(config: Config) -> Self {
        tracing::info!(target: Log::SlippiOnline, "Starting SlippiEXIDevice");

        let api_client = APIClient::new(&config.scm.slippi_semver);

        let user_manager = UserManager::new(
            api_client.clone(),
            config.paths.user_config_folder.clone().into(),
            config.scm.slippi_semver.clone(),
        );

        let game_reporter = GameReporter::new(
            api_client.clone(),
            user_manager.clone(),
            config.paths.iso.clone(),
            config.paths.user_config_folder.clone().into(),
        );

        // Playback has no need to deal with this.
        // (We could maybe silo more?)
        #[cfg(not(feature = "playback"))]
        user_manager.watch_for_login();

        Self {
            config,
            game_reporter,
            user_manager,
            jukebox: None,
            discord_rpc: None,
        }
    }

    /// Stubbed for now, but this would get called by the C++ EXI device on DMAWrite.
    pub fn dma_write(&mut self, _address: usize, _size: usize) {}

    /// Receives a chunk of the replay event stream from the C++ side and
    /// fans it out to everything interested in it.
    pub fn push_replay_data(&mut self, data: &[u8]) {
        self.game_reporter.push_replay_data(data);

        if let Some(discord_rpc) = &self.discord_rpc {
            discord_rpc.push_replay_data(data);
        }
    }

    /// Stubbed for now, but this would get called by the C++ EXI device on DMARead.
    pub fn dma_read(&mut self, _address: usize, _size: usize) {}

    /// Configures a new Jukebox, or ensures an existing one is dropped if it's being disabled.
    pub fn configure_jukebox(&mut self, config: JukeboxConfiguration) {
        if let JukeboxConfiguration::Stop = config {
            self.jukebox = None;
            return;
        }

        if self.jukebox.is_some() {
            tracing::warn!(target: Log::SlippiOnline, "Jukebox is already active");
            return;
        }

        if let JukeboxConfiguration::Start {
            initial_dolphin_system_volume,
            initial_dolphin_music_volume,
        } = config
        {
            match Jukebox::new(
                self.config.paths.iso.clone(),
                initial_dolphin_system_volume,
                initial_dolphin_music_volume,
            ) {
                Ok(jukebox) => {
                    self.jukebox = Some(jukebox);
                },

                Err(e) => tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?e,
                    "Failed to start Jukebox"
                ),
            }
        }
    }

    /// Configures Discord Rich Presence, or drops an existing handler if it's
    /// being disabled.
    pub fn configure_discord_rpc(&mut self, config: DiscordRpcConfiguration) {
        let DiscordRpcConfiguration::Start { show_rank } = config else {
            self.discord_rpc = None;
            return;
        };

        if self.discord_rpc.is_none() {
            match DiscordHandler::new(Some(self.user_manager.clone())) {
                Ok(handler) => {
                    self.discord_rpc = Some(handler);
                },

                Err(e) => {
                    tracing::error!(
                        target: Log::SlippiOnline,
                        error = ?e,
                        "Failed to start Discord Rich Presence"
                    );

                    return;
                },
            }
        }

        if let Some(discord_rpc) = &self.discord_rpc {
            discord_rpc.set_show_rank(show_rank);
        }
    }

    /// Forwards a matchmaking-state snapshot to the Discord presence thread, if active.
    pub fn update_matchmaking_state(&self, process_state: u8, online_mode: u8, opponent_name: String, opponent_rank: i8) {
        if let Some(discord_rpc) = &self.discord_rpc {
            discord_rpc.update_matchmaking_state(process_state, online_mode, non_empty(opponent_name), opponent_rank);
        }
    }

    /// Forwards a scene + character-select snapshot to the Discord presence thread, if active.
    pub fn update_scene_state(&self, major_scene: u8, minor_scene: u8, char_ids: [u8; 4], local_port: u8, stage_id: u8) {
        if let Some(discord_rpc) = &self.discord_rpc {
            discord_rpc.update_scene_state(major_scene, minor_scene, char_ids, local_port, stage_id);
        }
    }
}
