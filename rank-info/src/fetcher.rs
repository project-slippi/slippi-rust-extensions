use std::sync::{Arc, Mutex};

use slippi_gg_api::APIClient;
use slippi_user::UserManager;

use super::{Message, RankInfo, RankManager, RankManagerData};

use dolphin_integrations::Log;
use serde_json::{json, Value};
use std::sync::mpsc::Receiver;
use thiserror::Error;

#[derive(Debug)]
pub struct RankInfoFetcher {
    api_client: APIClient,
    user_manager: UserManager,
    rank_data: Arc<Mutex<RankManagerData>>,
}

impl RankInfoFetcher {
    pub fn new(api_client: APIClient, user_manager: UserManager, rank_data: Arc<Mutex<RankManagerData>>) -> Self {
        Self {
            api_client,
            user_manager,
            rank_data,
        }
    }

    pub fn fetch_user_rank(&self, connect_code: &str) -> Result<RankInfo, GetRankErrorKind> {
        match execute_rank_query(&self.api_client, connect_code) {
            Ok(value) => {
                let rank_response: Result<RankInfoAPIResponse, serde_json::Error> = serde_json::from_str(&value);
                match rank_response {
                    Ok(rank_resp) => {
                        let mut rank_data = self.rank_data.lock().unwrap();
                        rank_data.previous_rank = rank_data.current_rank.clone();

                        let prev_rank_data = match rank_data.clone().previous_rank {
                            Some(rank) => rank,
                            None => RankInfo {
                                rank: 0,
                                rating_ordinal: 0.0,
                                global_placing: 0,
                                regional_placing: 0,
                                rating_update_count: 0,
                                rank_change: 0,
                                rating_change: 0.0,
                            },
                        };
                        tracing::info!(target: Log::SlippiOnline, "prev rank: {0}", prev_rank_data.rank);
                        tracing::info!(target: Log::SlippiOnline, "prev rating: {0}", prev_rank_data.rating_ordinal);
                        tracing::info!(target: Log::SlippiOnline, "prev update count: {0}", prev_rank_data.rating_update_count);

                        let has_cached_rating = prev_rank_data.rating_ordinal != 0.0;
                        let has_cached_rank = prev_rank_data.rank != 0;

                        let rating_change: f32 = if has_cached_rating {
                            rank_resp.rating_ordinal - prev_rank_data.rating_ordinal
                        } else {
                            0.0
                        };

                        let curr_rating_ordinal = if !has_cached_rating {
                            rank_resp.rating_ordinal
                        } else {
                            prev_rank_data.rating_ordinal
                        };

                        let curr_rank = RankManager::decide_rank(
                            rank_resp.rating_ordinal,
                            rank_resp.daily_global_placement.unwrap_or_default(),
                            rank_resp.daily_regional_placement.unwrap_or_default(),
                            rank_resp.rating_update_count,
                        ) as i8;

                        let rank_change: i8 = if has_cached_rank {
                            curr_rank - prev_rank_data.rank as i8
                        } else {
                            0
                        };

                        rank_data.current_rank = Some(RankInfo {
                            rank: curr_rank - rank_change,
                            rating_ordinal: curr_rating_ordinal,
                            global_placing: match rank_resp.daily_regional_placement {
                                Some(global_placement) => global_placement,
                                None => 0,
                            },
                            regional_placing: match rank_resp.daily_regional_placement {
                                Some(regional_placement) => regional_placement,
                                None => 0,
                            },
                            rating_update_count: rank_resp.rating_update_count,
                            rating_change: rating_change,
                            rank_change: rank_change as i32,
                        });

                        // debug logs
                        let test = rank_data.current_rank.clone().unwrap();
                        tracing::info!(target: Log::SlippiOnline, "rank: {0}", test.rank);
                        tracing::info!(target: Log::SlippiOnline, "rating_ordinal: {0}", test.rating_ordinal);
                        tracing::info!(target: Log::SlippiOnline, "global_placing: {0}", test.global_placing);
                        tracing::info!(target: Log::SlippiOnline, "regional_placing: {0}", test.regional_placing);
                        tracing::info!(target: Log::SlippiOnline, "rating_update_count: {0}", test.rating_update_count);
                        tracing::info!(target: Log::SlippiOnline, "rating_change: {0}", test.rating_change);
                        tracing::info!(target: Log::SlippiOnline, "rank_change: {0}", test.rank_change);

                        Ok(RankInfo {
                            rank: 0,
                            rating_ordinal: 0.0,
                            global_placing: 0,
                            regional_placing: 0,
                            rating_update_count: 0,
                            rating_change: rating_change,
                            rank_change: rank_change as i32,
                        })
                    },
                    Err(_err) => Err(GetRankErrorKind::NotSuccessful("Failed to parse rank struct".to_owned())),
                }
            },
            Err(err) => Err(err),
        }
    }
}

pub fn run(fetcher: RankInfoFetcher, receiver: Receiver<Message>) {
    loop {
        match receiver.recv() {
            Ok(Message::FetchRank) => {
                let connect_code = fetcher.user_manager.get(|user| user.connect_code.clone());
                let _ = fetcher.fetch_user_rank(&connect_code);
            },

            Ok(Message::RankFetcherDropped) => {
                tracing::info!(target: Log::SlippiOnline, "Rank fetcher thread dropped");
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

#[derive(Error, Debug)]
pub enum GetRankErrorKind {
    #[error("GetRankErrorKind Network")]
    Net(slippi_gg_api::Error),

    #[error("GetRankErrorKind JSON")]
    JSON(serde_json::Error),

    #[error("GetRankErrorKind GraphQL")]
    GraphQL(String),

    #[error("GetRankErrorKind NotSuccessful")]
    NotSuccessful(String),
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RankInfoAPIResponse {
    #[serde(alias = "ratingOrdinal")]
    pub rating_ordinal: f32,

    #[serde(alias = "ratingUpdateCount")]
    pub rating_update_count: u32,

    #[serde(alias = "dailyGlobalPlacement", default)]
    pub daily_global_placement: Option<u8>,

    #[serde(alias = "dailyRegionalPlacement", default)]
    pub daily_regional_placement: Option<u8>,
}

const GRAPHQL_URL: &str = "https://internal.slippi.gg/graphql";

pub(crate) fn execute_rank_query(api_client: &APIClient, connect_code: &str) -> Result<String, GetRankErrorKind> {
    let profile_fields = r#"
        fragment profileFields on NetplayProfile {
            ratingOrdinal
            ratingUpdateCount
            dailyGlobalPlacement
            dailyRegionalPlacement
        }
    "#;

    let user_profile_page = r#"
        fragment userProfilePage on User {
            rankedNetplayProfile {
                ...profileFields
            }
        }
    "#;

    let query = format!(
        r#"
        {user_profile_page}
        {profile_fields}

        query UserProfilePageQuery($cc: String, $uid: String) {{
            getUser(fbUid: $uid, connectCode: $cc) {{
                ...userProfilePage
            }}
        }}
    "#
    );

    let variables = Some(json!({
        "cc": connect_code,
        "uid": connect_code
    }));

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
        .map_err(GetRankErrorKind::Net)?;

    let response_body = response.into_string().unwrap_or_else(|e| format!("Error: {}", e));

    // Parse the response JSON
    let response_json: Value = serde_json::from_str(&response_body).map_err(GetRankErrorKind::JSON)?;

    // Check for GraphQL errors
    if let Some(errors) = response_json.get("errors") {
        if errors.is_array() && !errors.as_array().unwrap().is_empty() {
            let error_message = serde_json::to_string_pretty(errors).unwrap();
            return Err(GetRankErrorKind::GraphQL(error_message));
        }
    }

    if let Some(data) = response_json.get("data") {
        if let Some(get_user) = data.get("getUser") {
            if let Some(profile) = get_user.get("rankedNetplayProfile") {
                return Ok(profile.to_string());
            }
            return Err(GetRankErrorKind::NotSuccessful("rankedNetplayProfile".into()));
        }
        return Err(GetRankErrorKind::NotSuccessful("getUser".into()));
    } else {
        Err(GetRankErrorKind::GraphQL(
            "No 'data' field in the GraphQL response.".to_string(),
        ))
    }
}
