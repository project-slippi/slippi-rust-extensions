//! This module contains data models and helper methods for handling user authentication and
//! interaction from within Slippi Dolphin.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use slippi_gg_api::APIClient;

mod chat;
pub use chat::DEFAULT_CHAT_MESSAGES;

mod direct_codes;
use direct_codes::DirectCodes;

mod rank;
use rank::RankData;
pub use rank::{FetchStatus, RankInfo};

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

    #[serde(alias = "ranked_ordinal")]
    pub ranked_ordinal: f32,

    #[serde(alias = "ranked_global_placing")]
    pub ranked_global_placing: u16,

    #[serde(alias = "ranked_local_placing")]
    pub ranked_local_placing: u16,

    #[serde(alias = "ranked_rating_update_count")]
    pub ranked_rating_update_count: u32,
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
    user_json_path: Arc<PathBuf>,
    pub direct_codes: DirectCodes,
    pub teams_direct_codes: DirectCodes,
    slippi_semver: String,
    watcher: Arc<Mutex<UserInfoWatcher>>,
    rank_data: Arc<Mutex<RankData>>,
    rank_request_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl UserManager {
    /// Creates and returns a new `UserManager` instance.
    ///
    /// This accepts a `PathBuf` specifying the folder where user files (e.g, `user.json`)
    /// live. This is an OS-specific value and we currently need to share it with Dolphin,
    /// so this should be passed via the FFI layer. In the future, we may be able to remove
    /// this restriction via some assumptions.
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
        let watcher = Arc::new(Mutex::new(UserInfoWatcher::new()));
        let rank_data = Arc::new(Mutex::new(RankData::default()));
        let rank_request_thread = Arc::new(Mutex::new(None));

        Self {
            api_client,
            user,
            user_json_path,
            direct_codes,
            teams_direct_codes,
            slippi_semver,
            watcher,
            rank_data,
            rank_request_thread,
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
        attempt_login(&self.api_client, &self.user, &self.user_json_path, &self.slippi_semver)
    }

    /// Kicks off a background handler for processing user authentication.
    pub fn watch_for_login(&self) {
        let mut watcher = self.watcher.lock().expect("Unable to acquire user watcher lock");

        watcher.watch_for_login(
            self.api_client.clone(),
            self.user_json_path.clone(),
            self.user.clone(),
            &self.slippi_semver,
        );
    }

    /// Pops open a browser window for the older authentication flow. This is less encountered by
    /// users as time goes on, but may still be used.
    pub fn open_login_page(&self) {
        let path_ref = self.user_json_path.as_path();

        if let Some(path) = path_ref.to_str() {
            let url = format!("https://slippi.gg/online/enable?path={path}");

            tracing::info!("[User] Login at path: {}", url);

            if let Err(error) = open::that_detached(&url) {
                tracing::error!(?error, ?url, "Failed to open login page");
            }
        } else {
            // This should never really happen, but it's conceivable that some odd unicode path
            // errors could happen... so just dump a log I guess.
            tracing::warn!(?path_ref, "Unable to convert user.json path to UTF-8 string");
        }
    }

    /// Pops open a browser window for the update URL. This is less encountered by users as time goes
    /// by, but still used.
    pub fn update_app(&self) -> bool {
        if let Err(error) = open::that_detached("https://slippi.gg/downloads?update=true") {
            tracing::error!(?error, "Failed to open update URL");
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
    pub fn current_rank_and_status(&self) -> (Option<RankInfo>, FetchStatus) {
        let data = self.rank_data.lock().unwrap();
        (data.current_rank.clone(), data.fetch_status.clone())
    }

    /// Fetches the match result for a given match ID.
    ///
    /// This will spin up a background thread to fetch the match result
    /// and update the rank data accordingly. If a background thread is already
    /// running, this will not start a new one.
    pub fn fetch_match_result(&self, match_id: String) {
        let mut thread = self.rank_request_thread.lock().unwrap();

        // If a user leaves and re-enters the CSS while a request is ongoing, we
        // don't want to fire up multiple threads and issue multiple requests: limit
        // things to one background thread at a time.
        if thread.is_some() && !thread.as_ref().unwrap().is_finished() {
            return;
        }

        let api_client = self.api_client.clone();
        let (uid, play_key) = self.get(|user| (user.uid.clone(), user.play_key.clone()));
        let data = self.rank_data.clone();

        let background_thread = thread::Builder::new()
            .name("RankMatchResultThread".into())
            .spawn(move || {
                rank::run_match_result(api_client, match_id, uid, play_key, data);
            })
            .expect("Failed to spawn RankMatchResultThread.");

        *thread = Some(background_thread);
    }

    /// Logs the current user out and removes their `user.json` from the filesystem.
    pub fn logout(&mut self) {
        self.rank_data = Arc::new(Mutex::new(RankData::default()));
        self.rank_request_thread = Arc::new(Mutex::new(None));
        self.set(|user| *user = UserInfo::default());

        if let Err(error) = std::fs::remove_file(self.user_json_path.as_path()) {
            tracing::error!(?error, "Failed to remove user.json on logout");
        }

        let mut watcher = self.watcher.lock().expect("Unable to acquire watcher lock on user logout");

        watcher.logout();
    }
}

/// Checks for the existence of a `user.json` file and, if found, attempts to load and parse it.
///
/// This returns a `bool` value so that the background thread can know whether to stop checking.
fn attempt_login(api_client: &APIClient, user: &Arc<Mutex<UserInfo>>, user_json_path: &PathBuf, slippi_semver: &str) -> bool {
    match std::fs::read_to_string(user_json_path) {
        Ok(contents) => match serde_json::from_str::<UserInfo>(&contents) {
            Ok(mut info) => {
                info.sanitize();

                let uid = info.uid.clone();
                {
                    let mut lock = user.lock().expect("Unable to lock user in attempt_login");

                    *lock = info;
                }

                overwrite_from_server(api_client, user, uid, slippi_semver);
                return true;
            },

            // JSON parsing error
            Err(error) => {
                tracing::error!(?error, "Unable to parse user.json");
                return false;
            },
        },

        // Filesystem I/O error
        Err(error) => {
            // A not-found file just means they haven't logged in yet... presumably.
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::error!(?error, "Unable to read user.json");
            }

            return false;
        },
    }
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
    pub global_placing: u16,

    #[serde(alias = "dailyRegionalPlacement")]
    pub regional_placing: u16,

    #[serde(alias = "ratingUpdateCount")]
    pub rating_update_count: u32,
}

/// Calls out to the Slippi server and fetches the user info, patching up the user info object
/// with any returned information.
fn overwrite_from_server(api_client: &APIClient, user: &Arc<Mutex<UserInfo>>, uid: String, slippi_semver: &str) {
    let is_beta = match slippi_semver.contains("beta") {
        true => "-beta",
        false => "",
    };

    // @TODO: Switch this to a GraphQL call? Likely a Fizzi/Nikki task.
    let url = format!("{USER_API_URL}{is_beta}/{uid}?additionalFields=chatMessages,rank");

    tracing::warn!(?url, "Fetching user info");

    match api_client.get(&url).call() {
        Ok(response) => match response.into_string() {
            Ok(body) => match serde_json::from_str::<UserInfoAPIResponse>(&body) {
                Ok(info) => {
                    let mut lock = user.lock().expect("Unable to lock user in attempt_login");

                    lock.uid = info.uid;
                    lock.display_name = info.display_name;
                    lock.connect_code = info.connect_code;
                    lock.latest_version = info.latest_version;
                    lock.chat_messages = Some(info.chat_messages);
                    lock.ranked_ordinal = info.rank.rating_ordinal;
                    lock.ranked_global_placing = info.rank.global_placing;
                    lock.ranked_local_placing = info.rank.regional_placing;
                    lock.ranked_rating_update_count = info.rank.rating_update_count;

                    // TODO: Figure out how to get rank to rank module
                    // perhaps set up some kind of broadcast

                    (*lock).sanitize();
                },

                Err(error) => {
                    tracing::error!(?error, "Unable to deserialize user info API payload");
                },
            },

            // Failed to read into a string, usually an I/O error.
            Err(error) => {
                tracing::error!(?error, "Unable to read user info response body");
            },
        },

        // `error` is an enum, where one branch will contain the status code if relevant.
        // We log the debug representation to just see it all.
        Err(error) => {
            tracing::error!(?error, "API call for user info failed");
        },
    }
}
