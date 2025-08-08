//! This module provides an interface for fetching and vending
//! player rank updates for Dolphin to work with.

use std::sync::{Arc, Mutex};
use std::thread;

use dolphin_integrations::Log;
use slippi_gg_api::APIClient;
use slippi_user::UserManager;

mod fetcher;
use fetcher::RankData;
pub use fetcher::{FetchStatus, RankInfo};

mod rank;

/// An interface for checking and managing player rank.
#[derive(Debug)]
pub struct RankManager {
    api_client: APIClient,
    user_manager: UserManager,
    data: Arc<Mutex<RankData>>,
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl RankManager {
    /// Creates a new `RankManager`.
    pub fn new(api_client: APIClient, user_manager: UserManager) -> Self {
        tracing::info!(target: Log::SlippiOnline, "Initializing RankManager");

        Self {
            api_client,
            user_manager,
            data: Arc::new(Mutex::new(RankData::default())),
            thread: Arc::new(Mutex::new(None)),
        }
    }

    /// Fetches the match result for a given match ID.
    ///
    /// This will spin up a background thread to fetch the match result
    /// and update the rank data accordingly. If a background thread is already
    /// running, this will not start a new one.
    pub fn fetch_match_result(&self, match_id: String) {
        let mut thread = self.thread.lock().unwrap();

        // If a user leaves and re-enters the CSS while a request is ongoing, we
        // don't want to fire up multiple threads and issue multiple requests: limit
        // things to one background thread at a time.
        if thread.is_some() && !thread.as_ref().unwrap().is_finished() {
            return;
        }

        let api_client = self.api_client.clone();
        let (uid, play_key) = self.user_manager.get(|user| (user.uid.clone(), user.play_key.clone()));
        let data = self.data.clone();

        let background_thread = thread::Builder::new()
            .name("RankMatchResultThread".into())
            .spawn(move || {
                fetcher::run_match_result(api_client, match_id, uid, play_key, data);
            })
            .expect("Failed to spawn RankMatchResultThread.");

        *thread = Some(background_thread);
    }

    /// Gets the current rank state (even if blank), along with the current status of
    /// any ongoing fetch operations.
    pub fn current_rank_and_status(&self) -> (Option<RankInfo>, FetchStatus) {
        let data = self.data.lock().unwrap();
        (data.current_rank.clone(), data.fetch_status.clone())
    }

    /// Clears out any known rank data, typically for e.g user logout.
    pub fn clear(&mut self) {
        let mut data = self.data.lock().unwrap();
        data.fetch_status = FetchStatus::NotFetched;
        data.current_rank = None;
    }

    /// Sets the user's initial rank data from the server response.
    /// This is typically called during login to populate the current rank state.
    pub fn set_user_rank(&self, rating_ordinal: f32, global_placing: u16, regional_placing: u16, rating_update_count: u32) {
        let rank_idx = rank::decide(rating_ordinal, global_placing, regional_placing, rating_update_count) as i8;

        let rank_info = RankInfo {
            rank: rank_idx,
            rating_ordinal,
            global_placing,
            regional_placing,
            rating_update_count,
            rating_change: 0.0, // No change on initial load
            rank_change: 0,     // No change on initial load
        };

        let mut rank_data = self.data.lock().unwrap();
        rank_data.current_rank = Some(rank_info);
        rank_data.fetch_status = FetchStatus::Fetched;
    }
}
