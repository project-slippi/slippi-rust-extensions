//! This module houses the `SlippiEXIDevice`, which is in effect a "shadow subclass" of the C++
//! Slippi EXI device.
//!
//! What this means is that the Slippi EXI Device (C++) holds a pointer to the Rust
//! `SlippiEXIDevice` and forwards calls over the C FFI. This has a fairly clean mapping to "when
//! Slippi stuff is happening" and enables us to let the Rust side live in its own world.

use std::time::Duration;

use ureq::AgentBuilder;

use dolphin_integrations::Log;
use slippi_discord_rpc::DiscordHandler;
use slippi_game_reporter::GameReporter;
use slippi_jukebox::Jukebox;
use slippi_user::UserManager;

mod config;
pub use config::{Config, FilePathsConfig, SCMConfig};

/// Configuration instructions that the FFI layer uses to call over here.
#[derive(Debug)]
pub enum JukeboxConfiguration {
    Start {
        initial_dolphin_system_volume: u8,
        initial_dolphin_music_volume: u8,
    },

    Stop,
}

/// Configuration instructions that the FFI layer uses to call over here.
#[derive(Debug)]
pub enum DiscordHandlerConfiguration {
    Start {
        ram_offset: u8
    },

    Stop,
}

/// An EXI Device subclass specific to managing and interacting with the game itself.
#[derive(Debug)]
pub struct SlippiEXIDevice {
    config: Config,
    pub game_reporter: GameReporter,
    pub user_manager: UserManager,
    pub jukebox: Option<Jukebox>,
    pub discord_handler: Option<DiscordHandler>,
}

impl SlippiEXIDevice {
    /// Creates and returns a new `SlippiEXIDevice` with default values.
    ///
    /// At the moment you should never need to call this yourself.
    pub fn new(config: Config) -> Self {
        tracing::info!(target: Log::SlippiOnline, "Starting SlippiEXIDevice");

        // We set `max_idle_connections` to `5` to mimic how CURL was configured in
        // the old C++ logic. This gets cloned and passed down into modules so that
        // the underlying connection pool is shared.
        let http_client = AgentBuilder::new()
            .max_idle_connections(5)
            .timeout(Duration::from_millis(5000))
            .user_agent(&format!("SlippiDolphin/{} (Rust)", config.scm.slippi_semver))
            .build();

        let user_manager = UserManager::new(
            http_client.clone(),
            config.paths.user_json.clone().into(),
            config.scm.slippi_semver.clone(),
        );

        let game_reporter = GameReporter::new(http_client.clone(), user_manager.clone(), config.paths.iso.clone());

        // Playback has no need to deal with this.
        // (We could maybe silo more?)
        #[cfg(not(feature = "playback"))]
        user_manager.watch_for_login();

        Self {
            config,
            game_reporter,
            user_manager,
            jukebox: None,
            discord_handler: None
        }
    }

    /// Stubbed for now, but this would get called by the C++ EXI device on DMAWrite.
    pub fn dma_write(&mut self, _address: usize, _size: usize) {}

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

    /// Configures a new Discord handler, or ensures an existing one is dropped if it's being
    /// disabled.
    pub fn configure_discord_handler(&mut self, config: DiscordHandlerConfiguration) {
        if let DiscordHandlerConfiguration::Stop = config {
            self.discord_handler = None;
            return;
        }

        if self.discord_handler.is_some() {
            tracing::warn!(target: Log::SlippiOnline, "Discord handler is already running.");
            return;
        }

        if let DiscordHandlerConfiguration::Start {
            ram_offset
        } = config
        {
            match DiscordHandler::new(ram_offset) {
                Ok(handler) => {
                    self.discord_handler = Some(handler);
                },

                Err(e) => {
                    tracing::error!(
                        target: Log::SlippiOnline,
                        error = ?e,
                        "Failed to start Discord handler"
                    );
                }
            }
        }
    }
}
