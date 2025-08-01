use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use serde_json::json;

use dolphin_integrations::Log;
use slippi_gg_api::{APIClient, GraphQLError};
use slippi_user::UserManager;

use super::{FetchStatus, RankInfo, RankManagerData};

/// Any events we're listening for in the background thread.
#[derive(Clone, Copy, Debug)]
pub enum Message {
    FetchRank,
    RankFetcherDropped,
}

/// The core loop of the background thread that handles network requests
/// for checking player rank updates.
pub fn listen(
    api_client: APIClient,
    user_manager: UserManager,
    rank_data: Arc<Mutex<RankManagerData>>,
    receiver: Receiver<Message>,
) {
    loop {
        match receiver.recv() {
            Ok(Message::FetchRank) => {
                let connect_code = user_manager.get(|user| user.connect_code.clone());

                match fetch_rank(&api_client, &connect_code) {
                    Ok(response) => {
                        calculate_rank(&rank_data, response);
                    },

                    Err(error) => {
                        // Set fetch status to error
                        let mut data = rank_data.lock().unwrap();
                        data.fetch_status = FetchStatus::Error;

                        tracing::error!(
                            target: Log::SlippiOnline,
                            ?error,
                            "Failed to fetch rank"
                        );
                    },
                }
            },

            Ok(Message::RankFetcherDropped) => {
                tracing::info!(target: Log::SlippiOnline, "RankManagerNetworkThread dropped");
            },

            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    ?error,
                    "Failed to receive Message, thread will exit"
                );

                break;
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
fn calculate_rank(rank_data: &Arc<Mutex<RankManagerData>>, response: RankInfoAPIResponse) {
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

    rank_data.fetch_status = FetchStatus::Fetched;

    // debug logs
    let test = rank_data.current_rank.unwrap();
    tracing::info!(target: Log::SlippiOnline, "rank: {0}", test.rank);
    tracing::info!(target: Log::SlippiOnline, "rating_ordinal: {0}", test.rating_ordinal);
    tracing::info!(target: Log::SlippiOnline, "global_placing: {0}", test.global_placing);
    tracing::info!(target: Log::SlippiOnline, "regional_placing: {0}", test.regional_placing);
    tracing::info!(target: Log::SlippiOnline, "rating_update_count: {0}", test.rating_update_count);
    tracing::info!(target: Log::SlippiOnline, "rating_change: {0}", test.rating_change);
    tracing::info!(target: Log::SlippiOnline, "rank_change: {0}", test.rank_change);
}
