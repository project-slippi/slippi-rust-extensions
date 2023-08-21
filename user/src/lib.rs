//! This module contains data models and helper methods for handling user authentication
//! from within Slippi Dolphin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// use dolphin_integrations::Log;

mod chat;
pub use chat::DEFAULT_CHAT_MESSAGES;

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
            // if self.chat_messages.len() != 16 {
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
    user: Arc<Mutex<UserInfo>>,
    user_json_path: Arc<PathBuf>,
    watcher: Arc<Mutex<UserInfoWatcher>>,
}

impl UserManager {
    /// Creates and returns a new `UserManager` instance.
    ///
    /// This accepts a `PathBuf` specifying the folder where user files (e.g, `user.json`)
    /// live. This is an OS-specific value and we currently need to share it with Dolphin,
    /// so this should be passed via the FFI layer. In the future, we may be able to remove
    /// this restriction via some assumptions.
    pub fn new(user_json_path: PathBuf) -> Self {
        let user = Arc::new(Mutex::new(UserInfo::default()));
        let user_json_path = Arc::new(user_json_path);
        let watcher = Arc::new(Mutex::new(UserInfoWatcher::new()));

        Self {
            user,
            user_json_path,
            watcher,
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
    /// If, in the rare event that a Mutex lock could not be acquired (which should... never
    /// happen), this will call the provided closure with `&None` while logging the error.
    ///
    /// ```no_run
    /// use slippi_user::UserManager;
    ///
    /// fn inspect(manager: UserManager) {
    ///     let uid = manager.get(|user| user.uid);
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
    fn set<F>(&self, handler: F)
    where
        F: FnOnce(&mut UserInfo),
    {
        let mut lock = self.user.lock().expect("Unable to acquire user setter lock");

        handler(&mut lock);
    }

    /// Runs the `attempt_login` function on the calling thread. If you need this to run in the
    /// background, you want `watch_for_login` instead.
    pub fn attempt_login(&self) -> bool {
        attempt_login(&self.user, &self.user_json_path)
    }

    /// Kicks off a background handler for processing user authentication.
    pub fn watch_for_login(&self) {
        let mut watcher = self.watcher.lock().expect("Unable to acquire user watcher lock");

        watcher.watch_for_login(self.user_json_path.clone(), self.user.clone());
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

    /// Logs the current user out and removes their `user.json` from the filesystem.
    pub fn logout(&mut self) {
        self.set(|user| *user = UserInfo::default());

        if let Err(error) = std::fs::remove_file(self.user_json_path.as_path()) {
            tracing::error!(?error, "Failed to remove user.json on logout");
        }

        let mut watcher = self.watcher.lock().expect("Unable to acquire watcher lock on user logout");

        watcher.logout();
    }
}

/// This type manages access to user information, as well as any background thread watching
/// for `user.json` file existence.
#[derive(Debug)]
pub struct UserInfoWatcher {
    should_watch: Arc<AtomicBool>,
    watcher_thread: Option<thread::JoinHandle<()>>,
}

impl UserInfoWatcher {
    /// Initializes a new `UserInfoWatcher`. Call `watch_for_login` to kick things off.
    pub fn new() -> Self {
        Self {
            should_watch: Arc::new(AtomicBool::new(false)),
            watcher_thread: None,
        }
    }

    /// Spins up (or re-spins-up) the background watcher thread for the `user.json` file.
    pub fn watch_for_login(&mut self, user_json_path: Arc<PathBuf>, user: Arc<Mutex<UserInfo>>) {
        // If we're already watching, no-op out.
        if self.should_watch.load(Ordering::Relaxed) {
            return;
        }

        // Release (join) the existing thread, if we have one.
        self.release_thread();

        // Start the new thread~
        let should_watch = self.should_watch.clone();
        should_watch.store(true, Ordering::Relaxed);

        let watcher_thread = thread::Builder::new()
            .name("SlippiUserJSONWatcherThread".into())
            .spawn(move || loop {
                if !should_watch.load(Ordering::Relaxed) {
                    return;
                }

                if attempt_login(&user, &user_json_path) {
                    return;
                }

                thread::sleep(Duration::from_millis(500));
            })
            .expect("Failed to spawn SlippiUserJSONWatcherThread");

        self.watcher_thread = Some(watcher_thread);
    }

    /// On logout, we just need to stop the watcher thread. The thread will get
    /// restarted via some conditions elsewhere.
    fn logout(&mut self) {
        self.should_watch.store(false, Ordering::Relaxed);
    }

    /// Standard logic for popping the thread handle and joining it, logging on failure.
    fn release_thread(&mut self) {
        if let Some(watcher_thread) = self.watcher_thread.take() {
            if let Err(error) = watcher_thread.join() {
                tracing::error!(?error, "user.json background thread join failure");
            }
        }
    }
}

impl Drop for UserInfoWatcher {
    /// Cleans up the background thread that we use for watching `user.json` status.
    fn drop(&mut self) {
        self.release_thread();
    }
}

/// Checks for the existence of a `user.json` file and, if found, attempts to load and parse it.
///
/// This returns a `bool` value so that the background thread can know whether to stop checking.
fn attempt_login(user: &Arc<Mutex<UserInfo>>, user_json_path: &PathBuf) -> bool {
    match std::fs::read_to_string(user_json_path) {
        Ok(contents) => match serde_json::from_str::<UserInfo>(&contents) {
            Ok(mut info) => {
                info.sanitize();

                let uid = info.uid.clone();
                {
                    let mut lock = user.lock().expect("Unable to lock user in attempt_login");

                    *lock = info;
                }

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

/// The core payload that represents user information. This type is expected to conform
/// to the same definition that the remote server uses.
#[derive(Debug, Default, serde::Deserialize)]
pub struct APIResponse {
    pub uid: String,

    #[serde(alias = "displayName")]
    pub display_name: String,

    #[serde(alias = "connectCode")]
    pub connect_code: String,

    #[serde(alias = "latestVersion")]
    pub latest_version: String,

    #[serde(alias = "chatMessages")]
    pub chat_messages: Vec<String>,
}

/// Calls out to the Slippi server and fetches the user info, patching up the user info object
/// with any returned information.
fn overwrite_from_server(user: &Arc<Mutex<UserInfo>>, uid: String) {
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
            Ok(body) => match serde_json::from_str::<APIResponse>(&body) {
                Ok(info) => {
                    let mut lock = user.lock().expect("Unable to lock user in attempt_login");

                    lock.uid = info.uid;
                    lock.display_name = info.display_name;
                    lock.connect_code = info.connect_code;
                    lock.latest_version = info.latest_version;
                    lock.chat_messages = Some(info.chat_messages);

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
