//! This module houses the `SlippiEXIDevice`, which is in effect a "shadow subclass" of the C++
//! Slippi EXI device.
//!
//! What this means is that the Slippi EXI Device (C++) holds a pointer to the Rust
//! `SlippiEXIDevice` and forwards calls over the C FFI. This has a fairly clean mapping to "when
//! Slippi stuff is happening" and enables us to let the Rust side live in its own world.

use dolphin_integrations::Log;
use slippi_game_reporter::SlippiGameReporter;
use slippi_jukebox::Jukebox;
use slippi_user::UserManager;

/// An EXI Device subclass specific to managing and interacting with the game itself.
#[derive(Debug)]
pub struct SlippiEXIDevice {
    iso_path: String,
    pub game_reporter: SlippiGameReporter,
    pub user_manager: UserManager,
    jukebox: Option<Jukebox>,
}

impl SlippiEXIDevice {
    /// Creates and returns a new `SlippiEXIDevice` with default values.
    ///
    /// At the moment you should never need to call this yourself.
    pub fn new(iso_path: String, user_folder_path: String) -> Self {
        tracing::info!(target: Log::EXI, "Starting SlippiEXIDevice");

        let user_manager = UserManager::new(user_folder_path.into());

        let game_reporter = SlippiGameReporter::new(user_manager.clone(), iso_path.clone());

        // Playback has no need to deal with this.
        // (We could maybe silo more?)
        #[cfg(not(feature = "playback"))]
        user_manager.watch_for_login();

        Self {
            iso_path,
            game_reporter,
            user_manager,
            jukebox: None,
        }
    }

    /// Stubbed for now, but this would get called by the C++ EXI device on DMAWrite.
    pub fn dma_write(&mut self, _address: usize, _size: usize) {}

    /// Stubbed for now, but this would get called by the C++ EXI device on DMARead.
    pub fn dma_read(&mut self, _address: usize, _size: usize) {}

    /// Configures a new Jukebox, or ensures an existing one is dropped if it's being disabled.
    pub fn configure_jukebox(&mut self, is_enabled: bool, get_dolphin_volume_fn: slippi_jukebox::ForeignGetVolumeFn) {
        if !is_enabled {
            self.jukebox = None;
            return;
        }

        match Jukebox::new(self.iso_path.clone(), get_dolphin_volume_fn) {
            Ok(jukebox) => {
                self.jukebox = Some(jukebox);
            },

            Err(e) => tracing::error!(
                target: Log::EXI,
                error = ?e,
                "Failed to start Jukebox"
            ),
        }
    }

    pub fn jukebox_play_music(&mut self, hps_offset: u64, hps_length: usize) {
        if let Some(jukebox) = self.jukebox.as_mut() {
            if let Err(e) = jukebox.play_music(hps_offset, hps_length) {
                tracing::error!(
                    target: Log::EXI,
                    error = ?e,
                    "Failed to play jukebox song music"
                )
            }
        }
    }

    pub fn jukebox_stop_music(&mut self) {
        if let Some(jukebox) = self.jukebox.as_mut() {
            jukebox.stop_music();
        }
    }

    pub fn jukebox_set_music_volume(&mut self, volume: u8) {
        if let Some(jukebox) = self.jukebox.as_mut() {
            jukebox.set_music_volume(volume);
        }
    }
}
