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

    /// Spins up a background thread to fire off a rank fetch request.
    ///
    /// If a background thread is still ongoing, then this is a no-op. This can
    /// happen if the background thread is in a retry loop.
    pub fn fetch(&self) {
        let mut thread = self.thread.lock().unwrap();

        // If a user leaves and re-enters the CSS while a request is ongoing, we
        // don't want to fire up multiple threads and issue multiple requests: limit
        // things to one background thread at a time.
        if thread.is_some() && !thread.as_ref().unwrap().is_finished() {
            return;
        }

        let api_client = self.api_client.clone();
        let connect_code = self.user_manager.get(|user| user.connect_code.clone());
        let data = self.data.clone();

        // Set the fetching status synchronously so the game will immediately see it
        fetcher::set_status(&data, FetchStatus::Fetching);

        let background_thread = thread::Builder::new()
            .name("RankRequestThread".into())
            .spawn(move || {
                fetcher::run(api_client, connect_code, data);
            })
            .expect("Failed to spawn RankRequestThread.");

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
        data.previous_rank = None;
    }
}
