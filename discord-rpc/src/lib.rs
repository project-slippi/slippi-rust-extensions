//! Discord Rich Presence for Slippi Dolphin.

use std::sync::mpsc::{self, Sender};
use std::thread;

use dolphin_integrations::Log;
use slippi_user::UserManager;

mod errors;
use DiscordRpcError::*;
pub use errors::DiscordRpcError;

mod content;
mod melee;
mod menu;
mod presence;
mod scene;
mod text;

mod parser;
pub use parser::{EventParser, GameStartInfo, PlayerInfo, SlpEvent, raw_replay_stream};

pub(crate) type Result<T> = std::result::Result<T, DiscordRpcError>;

/// Messages handled by the presence thread.
#[derive(Debug)]
pub(crate) enum Message {
    /// A chunk of the replay event stream, fed to the parser.
    ReplayData(Vec<u8>),

    /// A matchmaking-state snapshot pushed from the C++ side each online frame.
    Matchmaking {
        process_state: u8,
        online_mode: u8,
        opponent_name: Option<String>,
        opponent_rank: i8,
    },

    /// A scene + character-select snapshot pushed from the C++ side each
    /// menu-frame heartbeat. Drives character-select and offline-scene presence.
    Scene {
        major: u8,
        minor: u8,
        char_ids: [u8; 4],
        local_port: u8,
        stage_id: u8,
    },

    /// The player's in-game rank-display setting changed.
    ShowRank(bool),

    /// The handle was dropped; shut the thread down.
    Shutdown,
}

/// The public handle for the Discord Rich Presence integration.
#[derive(Debug)]
pub struct DiscordHandler {
    sender: Sender<Message>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DiscordHandler {
    /// Spawns the presence thread and returns the handle.
    pub fn new(user_manager: Option<UserManager>) -> Result<Self> {
        tracing::info!(target: Log::DiscordRpc, "Initializing Discord Rich Presence");

        let (sender, receiver) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("SlippiDiscordRpc".to_string())
            .spawn(move || {
                if let Err(error) = presence::run(receiver, user_manager) {
                    tracing::error!(target: Log::DiscordRpc, ?error, "Discord presence thread encountered an error");
                }
            })
            .map_err(ThreadSpawn)?;

        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }

    /// Forwards a chunk of the replay event stream to the presence thread.
    pub fn push_replay_data(&self, data: &[u8]) {
        // Fired every online frame; a dead channel means the thread is already
        // gone, so we stay quiet here rather than spam the log on the hot path.
        let _ = self.sender.send(Message::ReplayData(data.to_vec()));
    }

    /// Forwards a Slippi matchmaking-state snapshot to the presence thread.
    pub fn update_matchmaking_state(&self, process_state: u8, online_mode: u8, opponent_name: Option<String>, opponent_rank: i8) {
        let message = Message::Matchmaking {
            process_state,
            online_mode,
            opponent_name,
            opponent_rank,
        };

        if let Err(error) = self.sender.send(message) {
            tracing::warn!(target: Log::DiscordRpc, ?error, "Unable to dispatch matchmaking state to presence thread");
        }
    }

    /// Forwards a Slippi scene + character-select snapshot to the presence thread.
    pub fn update_scene_state(&self, major: u8, minor: u8, char_ids: [u8; 4], local_port: u8, stage_id: u8) {
        let message = Message::Scene {
            major,
            minor,
            char_ids,
            local_port,
            stage_id,
        };

        if let Err(error) = self.sender.send(message) {
            tracing::warn!(target: Log::DiscordRpc, ?error, "Unable to dispatch scene state to presence thread");
        }
    }

    /// Mirrors the player's in-game rank-display setting
    /// (`SLIPPI_ENABLE_RANK_LOCAL`); when false, the rank badge is hidden.
    pub fn set_show_rank(&self, show_rank: bool) {
        if let Err(error) = self.sender.send(Message::ShowRank(show_rank)) {
            tracing::warn!(target: Log::DiscordRpc, ?error, "Unable to dispatch rank-display setting to presence thread");
        }
    }
}

impl Drop for DiscordHandler {
    fn drop(&mut self) {
        if let Err(error) = self.sender.send(Message::Shutdown) {
            tracing::error!(target: Log::DiscordRpc, ?error, "Failed to notify Discord presence thread of shutdown, join may hang");
        }

        if let Some(thread) = self.thread.take() {
            if let Err(error) = thread.join() {
                tracing::error!(target: Log::DiscordRpc, ?error, "Discord presence thread panicked");
            }
        }
    }
}
