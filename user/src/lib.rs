//! This module contains data models and helper methods for handling user authentication and
//! interaction from within Slippi Dolphin.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dolphin_integrations::Log;
use slippi_gg_api::APIClient;

mod chat;
pub use chat::DEFAULT_CHAT_MESSAGES;

mod direct_codes;
use direct_codes::DirectCodes;

mod rank_fetcher;
pub use rank_fetcher::RankFetchStatus;
use rank_fetcher::{RankFetcher, RankFetcherStatus, SlippiRank};

mod watcher;
use watcher::UserInfoWatcher;

const USER_API_URL: &'static str = "https://users-rest-dot-slippi.uc.r.appspot.com/user";

/// The core payload that represents user information. This type is expected to conform
/// to the same definition that the remote server uses.
#[derive(Debug, Default, serde::Deserialize)]
pub struct UserInfo {
    pub uid: String,

    #[serde(alias = "playKey")]
    pub play_key: String,

    #[serde(alias = "displayName")]
    pub display_name: String,

    #[serde(alias = "connectCode")]
    pub connect_code: String,

    #[serde(alias = "latestVersion")]
    pub latest_version: String,

    #[serde(alias = "chatMessages")]
    pub chat_messages: Option<Vec<String>>,
}

impl UserInfo {
    /// Common logic that we need in different deserialization cases (filesystem, network, etc).
    ///
    /// Mostly checks to make sure we're not loading or receiving anything undesired.
    pub fn sanitize(&mut self) {
        if self.chat_messages.is_none() || self.chat_messages.as_ref().unwrap().len() != 16 {
            self.chat_messages = Some(chat::default());
        }
    }
}

/// Represents a slice of rank information from the Slippi server.
#[derive(Clone, Copy, Debug, Default)]
pub struct RankInfo {
    pub rank: i8,
    pub rating_ordinal: f32,
    pub global_placing: u16,
    pub regional_placing: u16,
    pub rating_update_count: u32,
    pub rating_change: f32,
    pub rank_change: i8,
}

/// A thread-safe handle for the User Manager. This uses an `Arc` under the hood, so you don't
/// need to do so if you're storing it.
///
/// In the future, this type could probably switch to `Rc<RefCell<>>` instead of `Arc<Mutex<>>`,
/// but we should get further along in the port before doing so to avoid any ill assumptions about
/// where this stuff is called into from the C++ side.
#[derive(Clone, Debug)]
pub struct UserManager {
    api_client: APIClient,
    user: Arc<Mutex<UserInfo>>,
    rank: Arc<Mutex<RankInfo>>,
    user_json_path: Arc<PathBuf>,
    pub direct_codes: DirectCodes,
    pub teams_direct_codes: DirectCodes,
    slippi_semver: String,
    watcher: Arc<Mutex<UserInfoWatcher>>,
    rank_fetcher: RankFetcher,
}

impl UserManager {
    /// Creates and returns a new `UserManager` instance.
    ///
    /// This accepts a `PathBuf` specifying the folder where user files (e.g, `user.json`)
    /// live. This is an OS-specific value and we currently need to share it with Dolphin,
    /// so this should be passed via the FFI layer. In the future, we may be able to remove
    /// this restriction via some assumptions.
    ///
    // @TODO: The semver param here should get refactored away in time once we've ironed out
    // how some things get persisted from the Dolphin side. Not a big deal to thread it for now.
    pub fn new(api_client: APIClient, mut user_config_folder: PathBuf, slippi_semver: String) -> Self {
        let direct_codes = DirectCodes::load({
            let mut path = user_config_folder.clone();
            path.push("direct-codes.json");
            path
        });

        let teams_direct_codes = DirectCodes::load({
            let mut path = user_config_folder.clone();
            path.push("teams-codes.json");
            path
        });

        let user_json_path = Arc::new({
            user_config_folder.push("user.json");
            user_config_folder
        });

        let user = Arc::new(Mutex::new(UserInfo::default()));
        let rank = Arc::new(Mutex::new(RankInfo::default()));

        let watcher = Arc::new(Mutex::new(UserInfoWatcher::new()));
        let rank_fetcher = RankFetcher::new();

        Self {
            api_client,
            user,
            rank,
            user_json_path,
            direct_codes,
            teams_direct_codes,
            slippi_semver,
            watcher,
            rank_fetcher,
        }
    }

    /// User info is held behind a Mutex as we access it from multiple threads. To read data
    /// from the user info, you can pass a closure to this method to extract whatever you need. If
    /// the user is not authenticated, then the underlying user is `None` and the closure will
    /// receive that as an argument.
    ///
    /// This is slightly better ergonomics wise than dealing with locking all over the place, and
    /// allows batch retrieval of properties.
    ///
    /// ```no_run
    /// use slippi_user::UserManager;
    ///
    /// fn inspect(manager: UserManager) {
    ///     let uid = manager.get(|user| user.uid.clone());
    ///     println!("User ID: {}", uid);
    /// }
    /// ```
    pub fn get<F, R>(&self, handler: F) -> R
    where
        F: FnOnce(&UserInfo) -> R,
    {
        let lock = self.user.lock().expect("Unable to acquire user getter lock");

        handler(&lock)
    }

    /// As user info is held behind a Mutex, we need to lock it to alter data on it. This is
    /// a simple helper method for automating that - and as a bonus, it's easier to batch-set
    /// properties without locking multiple times.
    ///
    /// ```no_run
    /// use slippi_user::UserManager;
    ///
    /// fn update(manager: UserManager, uid: String) {
    ///     manager.set(move |user| {
    ///         user.uid = uid;
    ///     })
    /// }
    /// ```
    pub fn set<F>(&self, handler: F)
    where
        F: FnOnce(&mut UserInfo),
    {
        let mut lock = self.user.lock().expect("Unable to acquire user setter lock");

        handler(&mut lock);
    }

    /// Runs the `attempt_login` function on the calling thread. If you need this to run in the
    /// background, you want `watch_for_login` instead.
    pub fn attempt_login(&self) -> bool {
        attempt_login(
            &self.api_client,
            &self.user,
            &self.rank,
            &self.rank_fetcher.status,
            &self.user_json_path,
            &self.slippi_semver,
        )
    }

    /// Kicks off a background handler for processing user authentication.
    pub fn watch_for_login(&self) {
        let mut watcher = self.watcher.lock().expect("Unable to acquire user watcher lock");

        watcher.watch_for_login(
            &self.api_client,
            &self.user_json_path,
            &self.user,
            &self.rank,
            &self.rank_fetcher.status,
            &self.slippi_semver,
        );
    }

    /// Pops open a browser window for the older authentication flow. This is less encountered by
    /// users as time goes on, but may still be used.
    pub fn open_login_page(&self) {
        let path_ref = self.user_json_path.as_path();

        if let Some(path) = path_ref.to_str() {
            let url = format!("https://slippi.gg/online/enable?path={path}");

            tracing::info!(target: Log::SlippiOnline, "[User] Login at path: {}", url);

            if let Err(error) = open::that_detached(&url) {
                tracing::error!(target: Log::SlippiOnline, ?error, ?url, "Failed to open login page");
            }
        } else {
            // This should never really happen, but it's conceivable that some odd unicode path
            // errors could happen... so just dump a log I guess.
            tracing::warn!(target: Log::SlippiOnline, ?path_ref, "Unable to convert user.json path to UTF-8 string");
        }
    }

    /// Pops open a browser window for the update URL. This is less encountered by users as time goes
    /// by, but still used.
    pub fn update_app(&self) -> bool {
        if let Err(error) = open::that_detached("https://slippi.gg/downloads?update=true") {
            tracing::error!(target: Log::SlippiOnline, ?error, "Failed to open update URL");
            return false;
        }

        true
    }

    /// Returns whether we have an authenticated user - i.e, whether we were able
    /// to find/load/parse their `user.json` file.
    pub fn is_logged_in(&self) -> bool {
        self.get(|user| user.uid != "")
    }

    /// During matchmaking, we may opt to force-overwrite the latest version to
    /// account for errors that can happen when the user tries to update.
    pub fn overwrite_latest_version(&self, version: String) {
        self.set(|user| {
            user.latest_version = version;
        });
    }

    /// Gets the current rank state (even if blank), along with the current status of
    /// any ongoing fetch operations.
    pub fn current_rank_and_status(&self) -> (RankInfo, RankFetchStatus) {
        let data = self.rank.lock().unwrap();
        let status = self.rank_fetcher.status.get();

        (*data, status)
    }

    /// Instructs the rank manager to check for any rank updates.
    pub fn fetch_match_result(&self, match_id: String) {
        let client = self.api_client.clone();
        let (uid, play_key) = self.get(|user| (user.uid.clone(), user.play_key.clone()));
        let rank = self.rank.clone();

        self.rank_fetcher.fetch_match_result(client, match_id, uid, play_key, rank);
    }

    /// Logs the current user out and removes their `user.json` from the filesystem.
    pub fn logout(&mut self) {
        self.rank = Arc::new(Mutex::new(RankInfo::default()));
        self.set(|user| *user = UserInfo::default());

        if let Err(error) = std::fs::remove_file(self.user_json_path.as_path()) {
            tracing::error!(target: Log::SlippiOnline, ?error, "Failed to remove user.json on logout");
        }

        let mut watcher = self.watcher.lock().expect("Unable to acquire watcher lock on user logout");

        watcher.logout();
    }
}

/// Checks for the existence of a `user.json` file and, if found, attempts to load and parse it.
///
/// This returns a `bool` value so that the background thread can know whether to stop checking.
fn attempt_login(
    api_client: &APIClient,
    user: &Mutex<UserInfo>,
    rank: &Mutex<RankInfo>,
    rank_fetcher_status: &RankFetcherStatus,
    user_json_path: &PathBuf,
    slippi_semver: &str,
) -> bool {
    let mut success = false;

    match std::fs::read_to_string(user_json_path) {
        Ok(contents) => match serde_json::from_str::<UserInfo>(&contents) {
            Ok(mut info) => {
                info.sanitize();

                let uid = info.uid.clone();
                {
                    let mut lock = user.lock().expect("Unable to lock user in attempt_login");
                    *lock = info;
                }

                if let Err(error) = overwrite_from_server(api_client, user, rank, uid, slippi_semver) {
                    tracing::error!(target: Log::SlippiOnline, ?error, "Failed to log in via server");
                } else {
                    success = true;
                }
            },

            // JSON parsing error
            Err(error) => {
                tracing::error!(target: Log::SlippiOnline, ?error, "Unable to parse user.json");
            },
        },

        // Filesystem I/O error
        Err(error) => {
            // A not-found file just means they haven't logged in yet... presumably.
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::error!(target: Log::SlippiOnline, ?error, "Unable to read user.json");
            }
        },
    }

    // This is likely already set in this case, but it doesn't hurt to be thorough.
    if !success {
        rank_fetcher_status.set(RankFetchStatus::Error);
    }

    success
}

/// The core payload that represents user information. This type is expected to conform
/// to the same definition that the remote server uses.
#[derive(Debug, Default, serde::Deserialize)]
struct UserInfoAPIResponse {
    pub uid: String,

    #[serde(alias = "displayName")]
    pub display_name: String,

    #[serde(alias = "connectCode")]
    pub connect_code: String,

    #[serde(alias = "latestVersion")]
    pub latest_version: String,

    #[serde(alias = "chatMessages")]
    pub chat_messages: Vec<String>,

    #[serde(alias = "rank")]
    pub rank: UserRankInfo,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct UserRankInfo {
    #[serde(alias = "ratingOrdinal")]
    pub rating_ordinal: f32,

    #[serde(alias = "dailyGlobalPlacement")]
    pub global_placing: Option<u16>,

    #[serde(alias = "dailyRegionalPlacement")]
    pub regional_placing: Option<u16>,

    #[serde(alias = "ratingUpdateCount")]
    pub rating_update_count: u32,
}

#[derive(Debug, thiserror::Error)]
enum APILoginError {
    #[error(transparent)]
    Client(ureq::Error),

    #[error(transparent)]
    IO(std::io::Error),
}

/// Calls out to the Slippi server and fetches the user info, patching up the user info object
/// with any returned information.
fn overwrite_from_server(
    api_client: &APIClient,
    user: &Mutex<UserInfo>,
    rank: &Mutex<RankInfo>,
    uid: String,
    slippi_semver: &str,
) -> Result<(), APILoginError> {
    let is_beta = match slippi_semver.contains("beta") {
        true => "-beta",
        false => "",
    };

    // @TODO: Switch this to a GraphQL call? Likely a Fizzi/Nikki task.
    let url = format!("{USER_API_URL}{is_beta}/{uid}?additionalFields=chatMessages,rank");

    tracing::warn!(target: Log::SlippiOnline, ?url, "Fetching user info");

    let info: UserInfoAPIResponse = api_client
        .get(&url)
        .call()
        .map_err(APILoginError::Client)?
        .into_json()
        .map_err(APILoginError::IO)?;

    let mut lock = user.lock().unwrap();
    lock.uid = info.uid;
    lock.display_name = info.display_name;
    lock.connect_code = info.connect_code;
    lock.latest_version = info.latest_version;
    lock.chat_messages = Some(info.chat_messages);
    (*lock).sanitize();

    let rank_idx = SlippiRank::decide(
        info.rank.rating_ordinal,
        info.rank.global_placing.unwrap_or(0),
        info.rank.regional_placing.unwrap_or(0),
        info.rank.rating_update_count,
    ) as i8;

    let mut lock = rank.lock().unwrap();

    *lock = RankInfo {
        rank: rank_idx,
        rating_ordinal: info.rank.rating_ordinal,
        global_placing: info.rank.global_placing.unwrap_or(0),
        regional_placing: info.rank.regional_placing.unwrap_or(0),
        rating_update_count: info.rank.rating_update_count,
        rating_change: 0.0, // No change on initial load
        rank_change: 0,     // No change on initial load
    };

    Ok(())
}
