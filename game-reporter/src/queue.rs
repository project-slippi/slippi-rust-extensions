//! This module implements the background queue for the Game Reporter.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};

use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};
use slippi_gg_api::APIClient;

use crate::types::{GameReport, GameReportRequestPayload, OnlinePlayMode};
use crate::{ProcessingEvent, StatusReportEvent};

const GRAPHQL_URL: &str = "https://internal.slippi.gg/graphql";

/// How many times a report should attempt to send.
const MAX_REPORT_ATTEMPTS: i32 = 5;

/// Expected response payload when saving a report to the server.
#[derive(Debug, serde::Deserialize)]
struct ReportResponse {
    success: bool,

    #[serde(rename = "uploadUrl")]
    upload_url: Option<String>,
}

/// An "inner" struct that holds shared points of data that we need to
/// access from multiple threads in this module.
///
/// By storing this separately, it both somewhat mimics how the original
/// C++ class was set up and makes life easier in terms of passing pieces
/// of data around various threads.
#[derive(Clone, Debug)]
pub struct GameReporterQueue {
    pub api_client: APIClient,
    pub iso_hash: Arc<Mutex<String>>,
    inner: Arc<Mutex<VecDeque<GameReport>>>,
}

impl GameReporterQueue {
    /// Initializes and returns a new game reporter.
    pub(crate) fn new(api_client: APIClient) -> Self {
        Self {
            api_client,
            iso_hash: Arc::new(Mutex::new(String::new())),
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Adds a new report to the back of the queue.
    ///
    /// (The processing thread pulls from the front)
    pub(crate) fn add_report(&self, report: GameReport) {
        match self.inner.lock() {
            Ok(mut lock) => {
                (*lock).push_back(report);
            },

            Err(error) => {
                // This should never happen.
                tracing::error!(target: Log::SlippiOnline, ?error, "Unable to lock queue, dropping report");
            },
        }
    }
}

pub(crate) fn run_report_match_status(api_client: APIClient, receiver: Receiver<StatusReportEvent>) {
    loop {
        // Watch for notification to do work
        match receiver.recv() {
            Ok(StatusReportEvent::ReportAvailable {
                uid,
                play_key,
                match_id,
                status,
            }) => {
                report_match_status(&api_client, uid, match_id, play_key, status);
            },

            Ok(StatusReportEvent::Shutdown) => {
                tracing::info!(target: Log::SlippiOnline, "Status report thread winding down");
                break;
            },

            // This should realistically never happen, since it means the Sender
            // that's held a level up has been dropped entirely - but we'll log
            // for the hell of it in case anyone's tweaking the logic.
            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    ?error,
                    "Failed to receive StatusReportEvent, thread will exit"
                );

                break;
            },
        }
    }
}

/// Report a match status update.
///
/// This doesn't necessarily need to be here, but it's easier to grok the codebase
/// if we keep all reporting network calls in one module.
pub fn report_match_status(api_client: &APIClient, uid: String, match_id: String, play_key: String, status: String) {
    let mutation = r#"
        mutation ($report: OnlineMatchStatusReportInput!) {
            reportOnlineMatchStatus (report: $report)
        }
    "#;

    let variables = Some(json!({
        "report": {
            "matchId": match_id,
            "fbUid": uid,
            "playKey": play_key,
            "status": status,
        }
    }));

    let res = execute_graphql_query(api_client, mutation, variables, Some("reportOnlineMatchStatus"));

    match res {
        Ok(value) if value == "true" => {
            tracing::info!(target: Log::SlippiOnline, "Successfully executed status report request: {status}")
        },
        Ok(value) => tracing::error!(target: Log::SlippiOnline, ?value, "Error executing status report request: {status}"),
        Err(error) => tracing::error!(target: Log::SlippiOnline, ?error, "Error executing status report request: {status}"),
    }
}

/// The main loop that processes reports.
pub(crate) fn run(reporter: GameReporterQueue, receiver: Receiver<ProcessingEvent>) {
    loop {
        // Watch for notification to do work
        match receiver.recv() {
            Ok(ProcessingEvent::ReportAvailable) => {
                process_reports(&reporter, ProcessingEvent::ReportAvailable);
            },

            Ok(ProcessingEvent::Shutdown) => {
                tracing::info!(target: Log::SlippiOnline, "Processing thread winding down");

                process_reports(&reporter, ProcessingEvent::Shutdown);

                break;
            },

            // This should realistically never happen, since it means the Sender
            // that's held a level up has been dropped entirely - but we'll log
            // for the hell of it in case anyone's tweaking the logic.
            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    ?error,
                    "Failed to receive ProcessingEvent, thread will exit"
                );

                break;
            },
        }
    }
}

/// Process jobs from the queue.
fn process_reports(queue: &GameReporterQueue, event: ProcessingEvent) {
    let Ok(iso_hash) = queue.iso_hash.lock() else {
        tracing::warn!(target: Log::SlippiOnline, "No ISO_HASH available");
        return;
    };

    let Ok(mut report_queue) = queue.inner.lock() else {
        tracing::warn!(target: Log::SlippiOnline, "Reporter Queue is dead");
        return;
    };

    // Process all reports currently in the queue.
    while !report_queue.is_empty() {
        // We only want to pop if we're successful in sending or if we encounter an error
        // (e.g, max attempts). We pass the locked queue over to work with the borrow checker
        // here, since otherwise we can't pop without some ugly block work to coerce letting
        // a mutable borrow drop.
        match try_send_next_report(&mut *report_queue, event, &queue.api_client, &iso_hash) {
            Ok(upload_url) => {
                // Pop the front of the queue. If we have a URL, chuck it all over
                // to the replay uploader.
                let report = report_queue.pop_front();

                tracing::info!(target: Log::SlippiOnline, "Successfully sent report, popping from queue");

                if let (Some(report), Some(upload_url)) = (report, upload_url) {
                    try_upload_replay_data(report.replay_data, upload_url, &queue.api_client);
                }

                thread::sleep(Duration::ZERO)
            },

            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    error = ?error.kind,
                    backoff = ?error.sleep_ms,
                    "Failed to send report"
                );

                if error.is_last_attempt {
                    tracing::error!(target: Log::SlippiOnline, "Hit max retry limit, dropping report");
                    let report = report_queue.pop_front(); // Remove the report so it no longer gets processed

                    // Tell player their report failed to send
                    if let Some(report) = report {
                        if report.online_mode == OnlinePlayMode::Ranked {
                            Dolphin::add_osd_message(
                                Color::Red,
                                OSDDuration::VeryLong,
                                "Failed to send game report. If you get this often, visit Slippi Discord for help.",
                            );
                        }
                    }
                }

                thread::sleep(error.sleep_ms)
            },
        }
    }
}

/// The true inner error, minus any metadata.
/// the compiler thinks the fields are unused, but they're not.
/// debug impls will render them over the Dolphin logging interface
/// and the compiler just doesn't see that.
#[derive(Debug)]
enum ReportSendErrorKind {
    #[allow(dead_code)]
    Net(slippi_gg_api::Error),
    #[allow(dead_code)]
    JSON(serde_json::Error),
    #[allow(dead_code)]
    GraphQL(String),
    #[allow(dead_code)]
    NotSuccessful(String),
}

/// Wraps errors that can occur during report sending.
#[derive(Debug)]
struct ReportSendError {
    is_last_attempt: bool,
    sleep_ms: Duration,
    kind: ReportSendErrorKind,
}

/// Builds a request payload and sends it.
///
/// If this is successful, it yields back an upload URL endpoint. This can be
/// passed to the upload call for processing.
fn try_send_next_report(
    queue: &mut VecDeque<GameReport>,
    event: ProcessingEvent,
    api_client: &APIClient,
    iso_hash: &str,
) -> Result<Option<String>, ReportSendError> {
    let report = (*queue).front_mut().expect("Reporter queue is empty yet it shouldn't be");

    report.attempts += 1;

    // If we're shutting the thread down, limit max attempts to just 1.
    let max_attempts = match event {
        ProcessingEvent::Shutdown => 1,
        _ => MAX_REPORT_ATTEMPTS,
    };

    let is_last_attempt = report.attempts >= max_attempts;

    let payload = GameReportRequestPayload::with(&report, iso_hash);

    let error_sleep_ms = match is_last_attempt {
        true => Duration::ZERO,
        false => Duration::from_millis((report.attempts as u64) * 100),
    };

    let mutation = r#"
        mutation ($report: OnlineGameReportInput!) {
            reportOnlineGame (report: $report) {
                success
                uploadUrl
            }
        }
    "#;

    let variables = Some(json!({
        "report": payload,
    }));

    // Call execute_graphql_query and get the response body as a String.
    let response_body =
        execute_graphql_query(api_client, mutation, variables, Some("reportOnlineGame")).map_err(|e| ReportSendError {
            is_last_attempt,
            sleep_ms: error_sleep_ms,
            kind: e,
        })?;

    // Now, parse the response JSON to get the data you need.
    let response: ReportResponse = serde_json::from_str(&response_body).map_err(|e| ReportSendError {
        is_last_attempt,
        sleep_ms: error_sleep_ms,
        kind: ReportSendErrorKind::JSON(e),
    })?;

    if !response.success {
        return Err(ReportSendError {
            is_last_attempt,
            sleep_ms: error_sleep_ms,
            kind: ReportSendErrorKind::NotSuccessful(response_body),
        });
    }

    Ok(response.upload_url)
}

/// Prepares and executes a GraphQL query.
fn execute_graphql_query(
    api_client: &APIClient,
    query: &str,
    variables: Option<Value>,
    field: Option<&str>,
) -> Result<String, ReportSendErrorKind> {
    // Prepare the GraphQL request payload
    let request_body = match variables {
        Some(vars) => json!({
            "query": query,
            "variables": vars,
        }),
        None => json!({
            "query": query,
        }),
    };

    // Make the GraphQL request
    let response = api_client
        .post(GRAPHQL_URL)
        .send_json(&request_body)
        .map_err(ReportSendErrorKind::Net)?;

    // Parse the response JSON
    let response_json: Value =
        serde_json::from_str(&response.into_string().unwrap_or_default()).map_err(ReportSendErrorKind::JSON)?;

    // Check for GraphQL errors
    if let Some(errors) = response_json.get("errors") {
        if errors.is_array() && !errors.as_array().unwrap().is_empty() {
            let error_message = serde_json::to_string_pretty(errors).unwrap();
            return Err(ReportSendErrorKind::GraphQL(error_message));
        }
    }

    // Return the data response
    if let Some(data) = response_json.get("data") {
        let result = match field {
            Some(field) => data.get(field).unwrap_or(data),
            None => data,
        };
        Ok(result.to_string())
    } else {
        Err(ReportSendErrorKind::GraphQL(
            "No 'data' field in the GraphQL response.".to_string(),
        ))
    }
}

/// Gzip compresses `input` data to `output` data.
fn compress_to_gzip(input: &[u8], output: &mut [u8]) -> Result<usize, std::io::Error> {
    let mut encoder = GzEncoder::new(output, Compression::default());
    encoder.write_all(input)?;

    let res = encoder.finish()?;

    Ok(res.len())
}

fn add_slp_header_and_footer(data: Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
    let guard = data.lock().unwrap();
    let raw_data_size = guard.len();
    let data_size_bytes = (raw_data_size as u32).to_be_bytes();

    let header = [b'{', b'U', 3, b'r', b'a', b'w', b'[', b'$', b'U', b'#', b'l'];
    let footer = [b'U', 8, b'm', b'e', b't', b'a', b'd', b'a', b't', b'a', b'{', b'}', b'}'];

    // Add header and footer to replay file
    header
        .iter()
        .chain(data_size_bytes.iter())
        .chain(guard.iter())
        .chain(footer.iter())
        .cloned()
        .collect()
}

/// Attempts to compress and upload replay data to the url at `upload_url`.
fn try_upload_replay_data(data: Arc<Mutex<Vec<u8>>>, upload_url: String, api_client: &APIClient) {
    let contents = add_slp_header_and_footer(data);

    let mut gzipped_data = vec![0u8; contents.len()]; // Resize to some initial size

    let res_size = match compress_to_gzip(&contents, &mut gzipped_data) {
        Ok(size) => size,

        Err(error) => {
            tracing::error!(target: Log::SlippiOnline, ?error, "Failed to compress replay");
            return;
        },
    };

    gzipped_data.resize(res_size, 0);

    let response = api_client
        .put(upload_url.as_str())
        .set("Content-Type", "application/octet-stream")
        .set("Content-Encoding", "gzip")
        .set("X-Goog-Content-Length-Range", "0,10000000")
        .send_bytes(&gzipped_data);

    if let Err(error) = response {
        tracing::error!(target: Log::SlippiOnline, ?error, "Failed to upload replay data",);
    }
}
