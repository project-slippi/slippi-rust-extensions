//! This module provides an interface for fetching and vending
//! player rank updates for Dolphin to work with.

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Sender, channel};
use std::thread;

use dolphin_integrations::Log;
use slippi_gg_api::APIClient;
use slippi_user::UserManager;

use crate::Message::*;

mod fetcher;
pub use fetcher::{FetchStatus, RankInfo};
use fetcher::{Message, RankData, listen};

mod rank;

/// An interface for checking and managing player rank.
#[derive(Debug)]
pub struct RankManager {
    tx: Sender<Message>,
    rank_data: Arc<Mutex<RankData>>,
}

impl RankManager {
    /// Creates a new `RankManager`. This spawns a background thread which listens
    /// for instructions and operates accordingly (e.g fetching rank updates).
    pub fn new(api_client: APIClient, user_manager: UserManager) -> Self {
        tracing::info!(target: Log::SlippiOnline, "Initializing RankManager");

        let (tx, rx) = channel::<Message>();
        let rank_data = Arc::new(Mutex::new(RankData::default()));
        let api_client_handle = api_client.clone();
        let user_manager_handle = user_manager.clone();
        let rank_data_handle = rank_data.clone();

        let _network_thread = thread::Builder::new()
            .name("RankManagerNetworkThread".into())
            .spawn(move || {
                listen(api_client_handle, user_manager_handle, rank_data_handle, rx);
            })
            .expect("Failed to spawn RankManagerNetworkThread.");

        Self { tx, rank_data }
    }

    /// Instructs the background thread to fire off a rank fetch request.
    pub fn fetch(&self) {
        if let Err(error) = self.tx.send(FetchRank) {
            tracing::error!(target: Log::SlippiOnline, ?error, "Unable to FetchRank");
        }
    }

    /// Gets the current rank state (even if blank), along with the current status of
    /// any ongoing fetch operations.
    pub fn current_rank_and_status(&self) -> (Option<RankInfo>, FetchStatus) {
        let data = self.rank_data.lock().unwrap();
        (data.current_rank.clone(), data.fetch_status.clone())
    }

    /// Clears out any known rank data, typically for e.g user logout.
    pub fn clear(&mut self) {
        let mut data = self.rank_data.lock().unwrap();
        data.fetch_status = FetchStatus::NotFetched;
        data.current_rank = None;
        data.previous_rank = None;
    }
}

impl Drop for RankManager {
    /// Notifies the background thread to shut down.
    fn drop(&mut self) {
        tracing::info!(target: Log::SlippiOnline, "Dropping RankManager");

        if let Err(e) = self.tx.send(Message::RankManagerDropped) {
            tracing::warn!(
                target: Log::SlippiOnline,
                "Failed to notify child thread that RankManager is dropping: {e}"
            );
        }
    }
}
