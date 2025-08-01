//! This module provides an interface for fetching and vending
//! player rank updates for Dolphin to work with.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use dolphin_integrations::Log;
use slippi_gg_api::APIClient;
use slippi_user::UserManager;

use crate::Message::*;

mod fetcher;
use fetcher::{Message, listen};

mod rank;

/// Represents a slice of rank information from the Slippi server.
#[derive(Clone, Copy, Debug, Default)]
pub struct RankInfo {
    pub rank: i8,
    pub rating_ordinal: f32,
    pub global_placing: u8,
    pub regional_placing: u8,
    pub rating_update_count: u32,
    pub rating_change: f32,
    pub rank_change: i32,
}

/// Represents current state of the rank flow.
///
/// Note that we mark this as C-compatible due to FFI usage.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub enum FetchStatus {
    #[default]
    NotFetched,
    Fetching,
    Fetched,
    Error,
}

#[derive(Debug, Clone, Default)]
struct RankManagerData {
    pub fetch_status: FetchStatus,
    pub current_rank: Option<RankInfo>,
    pub previous_rank: Option<RankInfo>,
}

#[derive(Debug)]
pub struct RankManager {
    tx: Sender<Message>,
    rank_data: Arc<Mutex<RankManagerData>>,
}

impl RankManager {
    /// Creates a new `RankManager`. This spawns a background thread which listens
    /// for instructions and operates accordingly (e.g fetching rank updates).
    pub fn new(api_client: APIClient, user_manager: UserManager) -> Self {
        tracing::info!(target: Log::SlippiOnline, "Initializing RankManager");

        let (tx, rx) = channel::<Message>();
        let rank_data = Arc::new(Mutex::new(RankManagerData::default()));
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

    pub fn fetch_rank(&self) {
        // Set fetch status to fetching
        let mut data = self.rank_data.lock().unwrap();
        data.fetch_status = FetchStatus::Fetching;

        // Send a message to the rank fetcher with the user's connect code
        let _ = self.tx.send(FetchRank);
    }

    pub fn get_rank(&self) -> Option<RankInfo> {
        self.rank_data.lock().unwrap().current_rank
    }

    pub fn get_rank_and_status(&self) -> (Option<RankInfo>, FetchStatus) {
        let data = self.rank_data.lock().unwrap();
        (data.current_rank.clone(), data.fetch_status.clone())
    }

    pub fn clear(&mut self) {
        let mut data = self.rank_data.lock().unwrap();
        data.current_rank = None;
        data.previous_rank = None;
    }
}

impl Drop for RankManager {
    fn drop(&mut self) {
        tracing::info!(target: Log::SlippiOnline, "Dropping Rank Fetcher");
        if let Err(e) = self.tx.send(Message::RankFetcherDropped) {
            tracing::warn!(
                target: Log::SlippiOnline,
                "Failed to notify child thread that Rank Fetcher is dropping: {e}"
            );
        }
    }
}
