use std::sync::{Arc, Mutex};
use std::thread;

use slippi_gg_api::APIClient;

use crate::RankInfo;

mod network;

mod rank;
pub use rank::SlippiRank;

/// Represents current state of the rank flow.
///
/// Note that we currently mark this as C-compatible due to FFI usage.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub enum RankFetchStatus {
    Fetching,
    Fetched,
    Error,
}

/// A newtype that exists simply to reduce boilerplate. This might also
/// get replaced by atomics, but I don't want to block launching.
#[derive(Clone, Debug)]
pub struct RankFetcherStatus(Arc<Mutex<RankFetchStatus>>);

impl RankFetcherStatus {
    /// Creates and returns a new status.
    ///
    /// This defaults to `Fetched` as we load initial rank data on client
    /// sign-in to begin with, meaning we should (theoretically, at least)
    /// always have some generic rank data to work with.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(RankFetchStatus::Fetched)))
    }

    /// Sets the underlying status.
    pub fn set(&self, status: RankFetchStatus) {
        let mut lock = self.0.lock().unwrap();
        *lock = status;
    }

    /// Gets the underlying status.
    pub fn get(&self) -> RankFetchStatus {
        let lock = self.0.lock().unwrap();
        *lock
    }
}

/// A type that holds and manages background rank update API calls.
#[derive(Clone, Debug)]
pub struct RankFetcher {
    pub status: RankFetcherStatus,
    request_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl RankFetcher {
    /// Creates and returns a new `RankFetcher`.
    pub fn new() -> Self {
        Self {
            status: RankFetcherStatus::new(),
            request_thread: Arc::new(Mutex::new(None)),
        }
    }

    /// Fetches the match result for a given match ID.
    ///
    /// This will spin up a background thread to fetch the match result
    /// and update the rank data accordingly. If a background thread is already
    /// running, this will not start a new one.
    pub fn fetch_match_result(
        &self,
        api_client: APIClient,
        match_id: String,
        uid: String,
        play_key: String,
        data: Arc<Mutex<RankInfo>>,
    ) {
        let mut thread = self.request_thread.lock().unwrap();

        // If a user leaves and re-enters the CSS while a request is ongoing, we
        // don't want to fire up multiple threads and issue multiple requests: limit
        // things to one background thread at a time.
        if thread.is_some() && !thread.as_ref().unwrap().is_finished() {
            return;
        }

        let status = self.status.clone();

        let background_thread = thread::Builder::new()
            .name("RankMatchResultThread".into())
            .spawn(move || {
                network::run_match_result(api_client, match_id, uid, play_key, status, data);
            })
            .expect("Failed to spawn RankMatchResultThread.");

        *thread = Some(background_thread);
    }
}
