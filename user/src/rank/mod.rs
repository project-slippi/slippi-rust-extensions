use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use serde_json::json;

use dolphin_integrations::Log;
use slippi_gg_api::{APIClient, GraphQLError};

mod rank;

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
}

/// Helper method for setting the fetch status.
pub fn set_status(data: &Mutex<RankData>, status: FetchStatus) {
    let mut lock = data.lock().unwrap();
    lock.fetch_status = status;
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

/// Updates the previous and current rank data based on the match result response.
fn update_rank(rank_data: &Arc<Mutex<RankData>>, response: MatchResultAPIResponse) {
    let mut rank_data = rank_data.lock().unwrap();

    // Grab the pre-match data and put it in previous.
    // It's possible that the previous will no longer match the prior previous rank
    // that was displayed, but I think that's okay because that would only happen
    // if another match was reported while we were in this one (abandonment) and we
    // want to correctly show the impact of the last match

    // Start loading in the pre-match values (previous rank)
    let mut rank_info = RankInfo {
        rating_ordinal: response.participant.pre_match_ordinal.unwrap_or(0.0),
        global_placing: response.participant.pre_match_daily_global_placement.unwrap_or(0),
        regional_placing: response.participant.pre_match_daily_regional_placement.unwrap_or(0),
        rating_update_count: response.participant.pre_match_rating_update_count.unwrap_or(0),
        rating_change: response.participant.post_match_rating_change.unwrap_or(0.0),
        ..Default::default()
    };

    // Determine the old rank based on the data pre-match data
    let prev_rank_idx = get_rank_idx_from_info(&rank_info);

    // Use rating change to update the rating_ordinal. Assume that the placements havent
    // changed since they only update once daily anyway. Also assume that update count
    // has incremented by 1. This could technically be incorrect but it would only matter
    // during placement matches so probably not a huge deal
    rank_info.rating_ordinal += rank_info.rating_change;
    rank_info.rating_update_count += 1;

    // Determine new rank index and rank change
    rank_info.rank = get_rank_idx_from_info(&rank_info);
    rank_info.rank_change = rank_info.rank - prev_rank_idx;

    // Load into rank_data
    rank_data.current_rank = Some(rank_info);
}

fn get_rank_idx_from_info(info: &RankInfo) -> i8 {
    rank::decide(
        info.rating_ordinal,
        info.global_placing,
        info.regional_placing,
        info.rating_update_count,
    ) as i8
}
