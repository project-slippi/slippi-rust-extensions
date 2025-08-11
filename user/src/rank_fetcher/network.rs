use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use serde_json::json;

use dolphin_integrations::Log;
use slippi_gg_api::{APIClient, GraphQLError};

use super::{RankFetchStatus, RankFetcherStatus, RankInfo, SlippiRank};

/// The core of the background thread that handles network requests
/// for checking player rank updates.
pub fn run_match_result(
    api_client: APIClient,
    match_id: String,
    uid: String,
    play_key: String,
    status: RankFetcherStatus,
    data: Arc<Mutex<RankInfo>>,
) {
    let mut retry_index = 0;

    status.set(RankFetchStatus::Fetching);

    loop {
        match fetch_match_result(&api_client, &match_id, &uid, &play_key) {
            Ok(response) => {
                // If the match hasn't been processed yet, wait and retry
                if response.status == MatchStatus::Assigned {
                    retry_index += 1;
                    if retry_index < 3 {
                        sleep(Duration::from_secs(2));
                        continue;
                    }
                }

                update_rank(&data, response);
                status.set(RankFetchStatus::Fetched);
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
                    status.set(RankFetchStatus::Error);
                    break;
                }

                let duration = Duration::from_secs(1);
                sleep(duration);
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
struct MatchResultParticipant {
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
    pub participant: MatchResultParticipant,
}

fn fetch_match_result(
    api_client: &APIClient,
    match_id: &str,
    uid: &str,
    play_key: &str,
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

    let response = api_client
        .graphql(query)
        .variables(variables)
        .data_field("/data/getRankedMatchPersonalResult")
        .send()?;

    Ok(response)
}

/// Updates the previous and current rank data based on the match result response.
fn update_rank(rank_data: &Mutex<RankInfo>, response: MatchResultAPIResponse) {
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
    let count_update = response.participant.post_match_rating_change.map_or(0, |_| 1);
    rank_info.rating_ordinal += rank_info.rating_change;
    rank_info.rating_update_count += count_update;

    // Determine new rank index and rank change
    rank_info.rank = get_rank_idx_from_info(&rank_info);
    rank_info.rank_change = rank_info.rank - prev_rank_idx;

    // Load into rank_data
    let mut rank_data = rank_data.lock().unwrap();
    *rank_data = rank_info;
}

fn get_rank_idx_from_info(info: &RankInfo) -> i8 {
    SlippiRank::decide(
        info.rating_ordinal,
        info.global_placing,
        info.regional_placing,
        info.rating_update_count,
    ) as i8
}
