//! This module contains data models and helper methods for handling user authentication
//! from within Slippi Dolphin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

// use dolphin_integrations::Log;

mod chat;
pub use chat::DEFAULT_CHAT_MESSAGES;

mod utils;
use utils::ThreadSafeWrapper;

const USER_API_URL: &'static str = "https://users-rest-dot-slippi.uc.r.appspot.com/user";

/// A type alias for how we hold `UserInfo` to share across threads.
type User = ThreadSafeWrapper<Option<UserInfo>>;

/// The core payload that represents user information. This type is expected to conform
/// to the same definition that the remote server uses.
#[derive(Debug, serde::Deserialize)]
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

    pub port: Option<i64>,

    #[serde(alias = "chatMessages", default = "chat::default")]
    pub chat_messages: Vec<String>,
}

impl UserInfo {
    /// Common logic that we need in different deserialization cases (filesystem, network, etc).
    ///
    /// Mostly checks to make sure we're not loading or receiving anything undesired.
    pub fn sanitize(&mut self) {
        if self.chat_messages.len() != 16 {
            self.chat_messages = chat::default();
        }
    }
}

/// This type manages access to user information, as well as any background thread watching
/// for `user.json` file existence.
#[derive(Debug)]
pub struct SlippiUserManager {
    user: User,
    user_json_path: Arc<PathBuf>,
    should_listen_for_auth: Arc<AtomicBool>,
    user_file_listener_thread: Option<thread::JoinHandle<()>>,
}

impl SlippiUserManager {
    /// Creates and returns a new `SlippiUserManager` instance.
    ///
    /// This accepts a `PathBuf` specifying the folder where user files (e.g, `user.json`)
    /// live. This is an OS-specific value and we currently need to share it with Dolphin,
    /// so this should be passed via the FFI layer. In the future, we may be able to remove
    /// this restriction via some assumptions.
    pub fn new(user_folder_path: PathBuf) -> Self {
        Self {
            user: User::new("user", None),
            user_json_path: Arc::new(user_folder_path.join("user.json")),
            should_listen_for_auth: Arc::new(AtomicBool::new(false)),
            user_file_listener_thread: None,
        }
    }

    /// Spins up (or re-spins-up) the background watcher thread for the `user.json` file.
    pub fn listen_for_login(&mut self) {
        // If we're already listening, no-op out.
        if self.should_listen_for_auth.load(Ordering::Relaxed) {
            return;
        }

        // Release (join) the existing thread, if we have one.
        self.release_thread();

        // Start the new thread~
        let should_listen_for_auth = self.should_listen_for_auth.clone();
        should_listen_for_auth.store(true, Ordering::Relaxed);

        let user = self.user.clone();
        let user_json_path = self.user_json_path.clone();

        let user_file_listener_thread = thread::Builder::new()
            .name("SlippiUserJSONWatcherThread".into())
            .spawn(move || loop {
                if !should_listen_for_auth.load(Ordering::Relaxed) {
                    return;
                }

                if attempt_login(&user, &user_json_path) {
                    return;
                }

                thread::sleep(Duration::from_millis(500));
            })
            .expect("Failed to spawn SlippiUserJSONWatcherThread");

        self.user_file_listener_thread = Some(user_file_listener_thread);
    }

    /// During matchmaking, we may opt to force-overwrite the latest version to
    /// account for errors that can happen when the user tries to update.
    pub fn overwrite_latest_version(&mut self, version: String) {
        self.user.with_mut(|user| {
            if let Some(user) = user {
                user.latest_version = version;
            }
        });
    }

    /// Returns whether we have an authenticated user - i.e, whether we were able
    /// to find/load/parse their `user.json` file.
    pub fn is_logged_in(&self) -> bool {
        self.user.get(|user| user.is_some()).is_some()
    }

    /// Logs the current user out and removes their `user.json` from the filesystem.
    pub fn logout(&mut self) {
        self.should_listen_for_auth.store(false, Ordering::Relaxed);
        self.user.set(None);

        if let Err(error) = std::fs::remove_file(self.user_json_path.as_path()) {
            tracing::error!(?error, "Failed to remove user.json on logout");
        }
    }

    /// Standard logic for popping the thread handle and joining it, logging on failure.
    fn release_thread(&mut self) {
        if let Some(user_file_listener_thread) = self.user_file_listener_thread.take() {
            if let Err(error) = user_file_listener_thread.join() {
                tracing::error!(?error, "user.json background thread join failure");
            }
        }
    }
}

impl Drop for SlippiUserManager {
    /// Cleans up the background thread that we use for watching `user.json` status.
    fn drop(&mut self) {
        self.release_thread();
    }
}

/// Checks for the existence of a `user.json` file and, if found, attempts to load and parse it.
///
/// This returns a `bool` value so that the background thread can know whether to stop checking.
fn attempt_login(user: &User, user_json_path: &PathBuf) -> bool {
    match std::fs::read_to_string(user_json_path) {
        Ok(contents) => match serde_json::from_str::<UserInfo>(&contents) {
            Ok(mut info) => {
                info.sanitize();

                let uid = info.uid.clone();

                user.set(Some(info));
                overwrite_from_server(user, uid);
                return true;
            },

            Err(error) => {
                tracing::error!(?error, "Unable to parse user.json");
                return false;
            },
        },

        Err(error) => {
            // A not-found file just means they haven't logged in yet... presumably.
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::error!(?error, "Unable to read user.json");
            }

            return false;
        },
    }
}

/// Pops open a browser window for the update URL. This is less encountered by users as time goes
/// by, but still used.
pub fn update_app() -> bool {
    if let Err(error) = open::that_detached("https://slippi.gg/downloads?update=true") {
        tracing::error!(?error, "Failed to open update URL");
        return false;
    }

    true
}

/// Calls out to the Slippi server and fetches the user info, patching up the user info object
/// with any returned information.
fn overwrite_from_server(user: &User, uid: String) {
    let is_beta = "";

    let url = format!("{USER_API_URL}{is_beta}/{uid}?additionalFields=chatMessages");

    tracing::warn!(?url, "Fetching user info");

    // This should eventually migrate up to a utils crate (along with GameReporter's agent), but
    // it's fine here for testing.
    let client = ureq::AgentBuilder::new()
        .user_agent("SlippiUserManager/0.1")
        .max_idle_connections(5)
        .timeout(Duration::from_millis(5000))
        .build();

    match client.get(&url).call() {
        Ok(response) => match response.into_string() {
            Ok(body) => match serde_json::from_str::<UserInfo>(&body) {
                Ok(mut info) => {
                    info.sanitize();
                    user.set(Some(info));
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
