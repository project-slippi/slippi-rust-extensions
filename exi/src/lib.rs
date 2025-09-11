//! This module houses the `SlippiEXIDevice`, which is in effect a "shadow subclass" of the C++
//! Slippi EXI device.
//!
//! What this means is that the Slippi EXI Device (C++) holds a pointer to the Rust
//! `SlippiEXIDevice` and forwards calls over the C FFI. This has a fairly clean mapping to "when
//! Slippi stuff is happening" and enables us to let the Rust side live in its own world.

use dolphin_integrations::Log;
use slippi_game_reporter::GameReporter;
use slippi_gg_api::APIClient;
use slippi_jukebox::Jukebox;
use slippi_playback::PlaybackEngine;
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
    pub playback: Option<PlaybackEngine>,
}

pub enum JukeboxConfiguration {
    Start {
        initial_dolphin_system_volume: u8,
        initial_dolphin_music_volume: u8,
    },
    Stop,
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

        let game_reporter = GameReporter::new(api_client.clone(), user_manager.clone(), config.paths.iso.clone());

        // Playback has no need to deal with this.
        // (We could maybe silo more?)
        #[cfg(not(feature = "playback"))]
        user_manager.watch_for_login();

        // Set up playback
        let playback = if cfg!(feature = "playback") {
            Some(PlaybackEngine::new_with_defaults(
                config.paths.playback_comm_file_path.clone(),
            ))
        } else {
            None
        };

        #[cfg(feature = "ishiiruka")]
        tracing::warn!(target: Log::SlippiOnline, "Ishiiruka feature is enabled");

        #[cfg(feature = "mainline")]
        tracing::warn!(target: Log::SlippiOnline, "Mainline feature is enabled");

        Self {
            config,
            game_reporter,
            user_manager,
            jukebox: None,
            playback: playback,
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
}
