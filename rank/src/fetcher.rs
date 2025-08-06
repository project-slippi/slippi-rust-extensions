use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use serde_json::json;

use dolphin_integrations::Log;
use slippi_gg_api::{APIClient, GraphQLError};

/// Represents a slice of rank information from the Slippi server.
#[derive(Clone, Copy, Debug, Default)]
pub struct RankInfo {
    pub rank: i8,
    pub rating_ordinal: f32,
    pub global_placing: u8,
    pub regional_placing: u8,
    pub rating_update_count: u32,
    pub rating_change: f32,
    pub rank_change: i32,
}

/// Represents current state of the rank flow.
///
/// Note that we mark this as C-compatible due to FFI usage.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub enum FetchStatus {
    #[default]
    NotFetched,
    Fetching,
    Fetched,
    Error,
}

/// Internal state representing player rank data, as well as the current
/// state of any network operations.
#[derive(Debug, Clone, Default)]
pub struct RankData {
    pub fetch_status: FetchStatus,
    pub current_rank: Option<RankInfo>,
    pub previous_rank: Option<RankInfo>,
}

/// Helper method for setting the fetch status.
pub fn set_status(data: &Mutex<RankData>, status: FetchStatus) {
    let mut lock = data.lock().unwrap();
    lock.fetch_status = status;
}

/// The core of the background thread that handles network requests
/// for checking player rank updates.
pub fn run(api_client: APIClient, connect_code: String, rank_data: Arc<Mutex<RankData>>) {
    let mut retry_index = 0;

    // Fetching state is set by the function initiating this async process to make
    // sure the status is set synchronously in case of any quick reads after the fetch

    loop {
        match fetch_rank(&api_client, &connect_code) {
            Ok(response) => {
                let rating_updated = calculate_rank(&rank_data, response);

                // If the rating hasn't been updated, we want to retry. This could
                // happen in the case where a match is a little late to be reported
                // on the server. This hopefully gives some time for our rank update
                // to be processed.
                if !rating_updated {
                    retry_index += 1;
                    if retry_index < 3 {
                        sleep(Duration::from_secs(2));
                        continue;
                    }
                }

                set_status(&rank_data, FetchStatus::Fetched);
                break;
            },

            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    ?error,
                    "Failed to fetch rank"
                );

                retry_index += 1;

                // Only set the error flag after multiple retries have failed(?)
                if retry_index >= 3 {
                    set_status(&rank_data, FetchStatus::Error);
                    break;
                }

                let duration = Duration::from_secs(1);
                sleep(duration);
            },
        }
    }
}

/// Expected return payload from the API.
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
struct RankInfoAPIResponse {
    #[serde(alias = "ratingOrdinal")]
    pub rating_ordinal: f32,

    #[serde(alias = "ratingUpdateCount")]
    pub rating_update_count: u32,

    #[serde(alias = "dailyGlobalPlacement")]
    pub daily_global_placement: Option<u8>,

    #[serde(alias = "dailyRegionalPlacement")]
    pub daily_regional_placement: Option<u8>,
}

/// Builds a query and fires off a rank info request.
fn fetch_rank(api_client: &APIClient, connect_code: &str) -> Result<RankInfoAPIResponse, GraphQLError> {
    let query = r#"
        query ($cc: String) {
            getUser(connectCode: $cc) {
                rankedNetplayProfile {
                    ratingOrdinal
                    ratingUpdateCount
                    dailyGlobalPlacement
                    dailyRegionalPlacement
                }
            }
        }
    "#;

    let variables = json!({ "cc": connect_code });

    let response: RankInfoAPIResponse = api_client
        .graphql(query)
        .variables(variables)
        .data_field("/data/getUser/rankedNetplayProfile")
        .send()?;

    Ok(response)
}

/// Calculates and stores any rank adjustments.
/// Returns true if the rating was updated, false otherwise.
fn calculate_rank(rank_data: &Arc<Mutex<RankData>>, response: RankInfoAPIResponse) -> bool {
    let mut rank_data = rank_data.lock().unwrap();
    rank_data.previous_rank = rank_data.current_rank;

    let prev_rank_data = rank_data.previous_rank.unwrap_or_default();

    tracing::info!(target: Log::SlippiOnline, "prev rank: {0}", prev_rank_data.rank);
    tracing::info!(target: Log::SlippiOnline, "prev rating: {0}", prev_rank_data.rating_ordinal);
    tracing::info!(target: Log::SlippiOnline, "prev update count: {0}", prev_rank_data.rating_update_count);

    let has_cached_rating = prev_rank_data.rating_ordinal != 0.0;
    let has_cached_rank = prev_rank_data.rank != 0;

    let rating_change: f32 = if has_cached_rating {
        response.rating_ordinal - prev_rank_data.rating_ordinal
    } else {
        0.0
    };

    let curr_rating_ordinal = if response.rating_ordinal != 0.0 {
        response.rating_ordinal
    } else if has_cached_rating {
        prev_rank_data.rating_ordinal
    } else {
        0.0
    };

    let curr_rank = crate::rank::decide(
        response.rating_ordinal,
        response.daily_global_placement.unwrap_or_default(),
        response.daily_regional_placement.unwrap_or_default(),
        response.rating_update_count,
    ) as i8;

    let rank_change = if has_cached_rank {
        curr_rank - prev_rank_data.rank as i8
    } else {
        0
    };

    rank_data.current_rank = Some(RankInfo {
        rank: curr_rank - rank_change,
        rating_ordinal: curr_rating_ordinal,
        global_placing: response.daily_regional_placement.unwrap_or_default(),
        regional_placing: response.daily_regional_placement.unwrap_or_default(),
        rating_update_count: response.rating_update_count,
        rating_change: rating_change,
        rank_change: rank_change as i32,
    });

    // debug logs
    let test = rank_data.current_rank.unwrap();
    tracing::info!(target: Log::SlippiOnline, "rank: {0}", test.rank);
    tracing::info!(target: Log::SlippiOnline, "rating_ordinal: {0}", test.rating_ordinal);
    tracing::info!(target: Log::SlippiOnline, "global_placing: {0}", test.global_placing);
    tracing::info!(target: Log::SlippiOnline, "regional_placing: {0}", test.regional_placing);
    tracing::info!(target: Log::SlippiOnline, "rating_update_count: {0}", test.rating_update_count);
    tracing::info!(target: Log::SlippiOnline, "rating_change: {0}", test.rating_change);
    tracing::info!(target: Log::SlippiOnline, "rank_change: {0}", test.rank_change);

    // Return true if the rating_update_count has changed
    response.rating_update_count != prev_rank_data.rating_update_count
}
