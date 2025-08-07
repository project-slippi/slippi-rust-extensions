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
    pub global_placing: u16,
    pub regional_placing: u16,
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
        match fetch_rank(&api_client, connect_code.clone()) {
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

/// The core of the background thread that handles network requests
/// for checking player rank updates.
pub fn run_match_result(api_client: APIClient, match_id: String, uid: String, play_key: String, rank_data: Arc<Mutex<RankData>>) {
    let mut retry_index = 0;

    loop {
        set_status(&rank_data, FetchStatus::Fetching);

        match fetch_match_result(&api_client, match_id.clone(), uid.clone(), play_key.clone()) {
            Ok(response) => {
                // If the match hasn't been processed yet, wait and retry
                if response.status == MatchStatus::Assigned {
                    retry_index += 1;
                    if retry_index < 3 {
                        sleep(Duration::from_secs(2));
                        continue;
                    }
                }

                update_rank(&rank_data, response);
                set_status(&rank_data, FetchStatus::Fetched);
                break;
            },

            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    ?error,
                    "Failed to fetch match result"
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
    pub daily_global_placement: Option<u16>,

    #[serde(alias = "dailyRegionalPlacement")]
    pub daily_regional_placement: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
struct MatchResultParticipantAPIResponse {
    #[serde(alias = "ordinal")]
    pub pre_match_ordinal: Option<f32>,

    #[serde(alias = "dailyGlobalPlacement")]
    pub pre_match_daily_global_placement: Option<u16>,

    #[serde(alias = "dailyRegionalPlacement")]
    pub pre_match_daily_regional_placement: Option<u16>,

    #[serde(alias = "ratingUpdateCount")]
    pub pre_match_rating_update_count: Option<u32>,

    #[serde(alias = "ratingChange")]
    pub post_match_rating_change: Option<f32>, // Null until the match is processed
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MatchStatus {
    Assigned,
    Complete,
    Abandoned,
    Orphaned,
    Terminated,
    Error,
    Unhandled,
}

impl Default for MatchStatus {
    fn default() -> Self {
        MatchStatus::Error // Default to error in case it's missing
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
struct MatchResultAPIResponse {
    #[serde(alias = "status")]
    pub status: MatchStatus,

    // Include the participant
    #[serde(alias = "participant")]
    pub participant: MatchResultParticipantAPIResponse,
}

/// Builds a query and fires off a rank info request.
fn fetch_rank(api_client: &APIClient, connect_code: String) -> Result<RankInfoAPIResponse, GraphQLError> {
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

fn fetch_match_result(
    api_client: &APIClient,
    match_id: String,
    uid: String,
    play_key: String,
) -> Result<MatchResultAPIResponse, GraphQLError> {
    let query = r#"
        query ($request: OnlineMatchRequestInput!) {
            getRankedMatchPersonalResult(request: $request) {
                status
                participant {
                    ordinal
                    dailyGlobalPlacement
                    dailyRegionalPlacement
                    ratingUpdateCount
                    ratingChange
                }
            }
        }
    "#;

    let variables = json!({
        "request": {
            "matchId": match_id,
            "fbUid": uid,
            "playKey": play_key,
        }
    });

    let response: MatchResultAPIResponse = api_client
        .graphql(query)
        .variables(variables)
        .data_field("/data/getRankedMatchPersonalResult")
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

/// Updates the previous and current rank data based on the match result response.
fn update_rank(rank_data: &Arc<Mutex<RankData>>, response: MatchResultAPIResponse) {
    let mut rank_data = rank_data.lock().unwrap();

    // Grab the pre-match data and put it in previous.
    // It's possible that the previous will no longer match the prior previous rank
    // that was displayed, but I think that's okay because that would only happen
    // if another match was reported while we were in this one (abandonment) and we
    // want to correctly show the impact of the last match
    let mut previous_rank = RankInfo {
        rating_ordinal: response.participant.pre_match_ordinal.unwrap_or(0.0),
        global_placing: response.participant.pre_match_daily_global_placement.unwrap_or(0),
        regional_placing: response.participant.pre_match_daily_regional_placement.unwrap_or(0),
        rating_update_count: response.participant.pre_match_rating_update_count.unwrap_or(0),
        ..Default::default()
    };

    // Determine the rank based on the data
    previous_rank.rank = crate::rank::decide(
        previous_rank.rating_ordinal,
        previous_rank.global_placing,
        previous_rank.regional_placing,
        previous_rank.rating_update_count,
    ) as i8;

    let rating_change = response.participant.post_match_rating_change.unwrap_or(0.0);

    // Use rating change to update the rating_ordinal. Assume that the placements havent
    // changed since they only update once daily anyway. Also assume that update count
    // has incremented by 1. This could technically be incorrect but it would only matter
    // during placement matches so probably not a huge deal
    let mut current_rank = RankInfo {
        rating_ordinal: previous_rank.rating_ordinal + rating_change,
        global_placing: previous_rank.global_placing,
        regional_placing: previous_rank.regional_placing,
        rating_update_count: previous_rank.rating_update_count + 1,
        rating_change,
        ..Default::default()
    };

    current_rank.rank = crate::rank::decide(
        current_rank.rating_ordinal,
        current_rank.global_placing,
        current_rank.regional_placing,
        current_rank.rating_update_count,
    ) as i8;

    current_rank.rank_change = current_rank.rank as i32 - previous_rank.rank as i32;

    rank_data.previous_rank = Some(previous_rank);
    rank_data.current_rank = Some(current_rank);
}
