use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use slippi_gg_api::APIClient;

use super::{attempt_login, UserInfo};

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
    pub fn watch_for_login(
        &mut self,
        api_client: APIClient,
        user_json_path: Arc<PathBuf>,
        user: Arc<Mutex<UserInfo>>,
        slippi_semver: &str,
    ) {
        // If we're already watching, no-op out.
        if self.should_watch.load(Ordering::Relaxed) {
            return;
        }

        // Release (join) the existing thread, if we have one.
        self.release_thread();

        // Start the new thread~
        let should_watch = self.should_watch.clone();
        should_watch.store(true, Ordering::Relaxed);

        // Create an owned String once we know we're actually launching the thread.
        let slippi_semver = slippi_semver.to_string();

        let watcher_thread = thread::Builder::new()
            .name("SlippiUserJSONWatcherThread".into())
            .spawn(move || loop {
                if !should_watch.load(Ordering::Relaxed) {
                    return;
                }

                if attempt_login(&api_client, &user, &user_json_path, &slippi_semver) {
                    return;
                }

                thread::sleep(Duration::from_millis(500));
            })
            .expect("Failed to spawn SlippiUserJSONWatcherThread");

        self.watcher_thread = Some(watcher_thread);
    }

    /// On logout, we just need to stop the watcher thread. The thread will get
    /// restarted via some conditions elsewhere.
    pub fn logout(&mut self) {
        self.should_watch.store(false, Ordering::Relaxed);
    }

    /// Standard logic for popping the thread handle and joining it, logging on failure.
    fn release_thread(&mut self) {
        self.should_watch.store(false, Ordering::Relaxed);
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
