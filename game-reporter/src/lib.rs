//! This could be rewritten down the road, but the goal is a 1:1 port right now,
//! not to rewrite the universe.

use std::ops::Deref;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

use ureq::Agent;

use dolphin_integrations::Log;
use slippi_user::UserManager;

mod iso_md5_hasher;

mod queue;
use queue::GameReporterQueue;

mod types;
pub use types::{GameReport, OnlinePlayMode, PlayerReport};

/// Events that we dispatch into the processing thread.
#[derive(Copy, Clone, Debug)]
pub(crate) enum ProcessingEvent {
    ReportAvailable,
    Shutdown,
}

/// Used to pass completion event data to a background processing thread.
#[derive(Clone, Debug)]
pub(crate) enum CompletionEvent {
    ReportAvailable {
        uid: String,
        play_key: String,
        match_id: String,
        end_mode: u8,
    },

    Shutdown,
}

/// The public interface for the game reporter service. This handles managing any
/// necessary background threads and provides hooks for instrumenting the reporting
/// process.
///
/// The inner `GameReporter` is shared between threads and manages field access via
/// internal Mutexes. We supply a channel to the processing thread in order to notify
/// it of new reports to process.
#[derive(Debug)]
pub struct GameReporter {
    user_manager: UserManager,
    iso_md5_hasher_thread: Option<thread::JoinHandle<()>>,
    queue_thread: Option<thread::JoinHandle<()>>,
    queue_thread_notifier: Sender<ProcessingEvent>,
    completion_thread: Option<thread::JoinHandle<()>>,
    completion_thread_notifier: Sender<CompletionEvent>,
    queue: GameReporterQueue,
    replay_data: Arc<Mutex<Vec<u8>>>,
}

impl GameReporter {
    /// Initializes and returns a new `GameReporter`.
    ///
    /// This spawns and manages a few background threads to handle things like
    /// report and upload processing, along with checking for troublesome ISOs.
    /// The core logic surrounding reports themselves lives a layer deeper in `GameReporter`.
    ///
    /// Currently, failure to spawn any thread should result in a crash - i.e, if we can't
    /// spawn an OS thread, then there are probably far bigger issues at work here.
    pub fn new(http_client: Agent, user_manager: UserManager, iso_path: String) -> Self {
        let queue = GameReporterQueue::new(http_client.clone());

        // This is a thread-safe "one time" setter that the MD5 hasher thread
        // will set when it's done computing.
        let iso_hash_setter = queue.iso_hash.clone();

        let iso_md5_hasher_thread = thread::Builder::new()
            .name("GameReporterISOHasherThread".into())
            .spawn(move || {
                iso_md5_hasher::run(iso_hash_setter, iso_path);
            })
            .expect("Failed to spawn GameReporterISOHasherThread.");

        let (queue_sender, queue_receiver) = mpsc::channel();
        let queue_thread_queue_handle = queue.clone();

        let queue_thread = thread::Builder::new()
            .name("GameReporterQueueProcessingThread".into())
            .spawn(move || {
                queue::run(queue_thread_queue_handle, queue_receiver);
            })
            .expect("Failed to spawn GameReporterQueueProcessingThread.");

        let (completion_sender, completion_receiver) = mpsc::channel();

        let completion_thread = thread::Builder::new()
            .name("GameReporterCompletionProcessingThread".into())
            .spawn(move || {
                queue::run_completion(http_client, completion_receiver);
            })
            .expect("Failed to spawn GameReporterCompletionProcessingThread.");

        Self {
            user_manager,
            queue,
            replay_data: Arc::new(Mutex::new(Vec::new())),
            queue_thread_notifier: queue_sender,
            queue_thread: Some(queue_thread),
            completion_thread_notifier: completion_sender,
            completion_thread: Some(completion_thread),
            iso_md5_hasher_thread: Some(iso_md5_hasher_thread),
        }
    }

    /// Currently unused.
    pub fn start_new_session(&mut self) {
        // Maybe we could do stuff here? We used to initialize gameIndex but
        // that isn't required anymore
    }

    /// Logs replay data that's passed to it.
    pub fn push_replay_data(&mut self, data: &[u8]) {
        if !data.is_empty() && data[0] == 0x35 {
            self.replay_data = Arc::new(Mutex::new(Vec::new()));
        }

        let mut guard = self.replay_data.lock().unwrap();
        guard.extend_from_slice(data);
    }

    /// Adds a report for processing and signals to the processing thread that there's
    /// work to be done.
    ///
    /// Note that when a new report is added, we transfer ownership of all current replay data
    /// to the game report itself. By doing this, we avoid needing to have a Mutex controlling
    /// access and pushing replay data as it comes in requires no locking.
    pub fn log_report(&mut self, mut report: GameReport) {
        report.replay_data = self.replay_data.clone();
        self.queue.add_report(report);

        if let Err(e) = self.queue_thread_notifier.send(ProcessingEvent::ReportAvailable) {
            tracing::error!(
                target: Log::SlippiOnline,
                error = ?e,
                "Unable to dispatch ReportAvailable notification"
            );
        }
    }

    /// Reports a match abandon event.
    pub fn report_abandonment(&self, match_id: String) {
        let (uid, play_key) = self.user_manager.get(|user| (user.uid.clone(), user.play_key.clone()));

        self.queue.report_abandonment(uid, play_key, match_id);
    }

    /// Dispatches a completion report to a background processing thread.
    pub fn report_completion(&self, match_id: String, end_mode: u8) {
        let (uid, play_key) = self.user_manager.get(|user| (user.uid.clone(), user.play_key.clone()));

        let event = CompletionEvent::ReportAvailable {
            uid,
            play_key,
            match_id,
            end_mode,
        };

        if let Err(e) = self.completion_thread_notifier.send(event) {
            tracing::error!(
                target: Log::SlippiOnline,
                error = ?e,
                "Unable to dispatch match completion notification"
            );
        }
    }
}

impl Deref for GameReporter {
    type Target = GameReporterQueue;

    /// Support dereferencing to the inner game reporter. This has a "subclass"-like
    /// effect wherein we don't need to duplicate methods on this layer.
    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

impl Drop for GameReporter {
    /// Joins the background threads when we're done, logging if
    /// any errors are encountered.
    fn drop(&mut self) {
        if let Some(queue_thread) = self.queue_thread.take() {
            if let Err(e) = self.queue_thread_notifier.send(ProcessingEvent::Shutdown) {
                tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?e,
                    "Failed to send shutdown notification to queue processing thread, may hang"
                );
            }

            if let Err(e) = queue_thread.join() {
                tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?e,
                    "Queue thread failure"
                );
            }
        }

        if let Some(completion_thread) = self.completion_thread.take() {
            if let Err(e) = self.completion_thread_notifier.send(CompletionEvent::Shutdown) {
                tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?e,
                    "Failed to send shutdown notification to completion processing thread, may hang"
                );
            }

            if let Err(e) = completion_thread.join() {
                tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?e,
                    "Completion thread failure"
                );
            }
        }

        if let Some(iso_md5_hasher_thread) = self.iso_md5_hasher_thread.take() {
            if let Err(e) = iso_md5_hasher_thread.join() {
                tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?e,
                    "ISO MD5 hasher thread failure"
                );
            }
        }
    }
}
