use slippi_gg_api::APIClient;
use slippi_user::UserManager;
use slippi_user::UserInfo;
use serde_json::{json, Value};

mod utils;
use utils::execute_graphql_query;

/// The true inner error, minus any metadata.
#[derive(Debug)]
enum GetRankErrorKind {
    Net(slippi_gg_api::Error),
    JSON(serde_json::Error),
    GraphQL(String),
    NotSuccessful(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlippiRank {
    Unranked,
    Bronze1,
    Bronze2,
    Bronze3,
    Silver1,
    Silver2,
    Silver3,
    Gold1,
    Gold2,
    Gold3,
    Platinum1,
    Platinum2,
    Platinum3,
    Diamond1,
    Diamond2,
    Diamond3,
    Master1,
    Master2,
    Master3,
    Grandmaster,
}

#[derive(Debug)]
pub struct RankInfo {
    rating_ordinal: f32,
    global_placing: u8,
    regional_placing: u8,
    rating_update_count: u8,
    rating_change: f32,
    rank_change: i8,
}

impl RankInfo {
    pub fn new(api_client: APIClient, user_manager: UserManager) -> Self {
        // TODO :: function for determining rank based on global / regional placement
        Self {
            rating_ordinal: 0.0,
            global_placing: 0,
            regional_placing: 0,
            rating_update_count: 0,
            rating_change: 0.0,
            rank_change: 0
        }
    }

    pub fn fetch_user_rank(api_client: APIClient, user: UserInfo) {
        let profile_fields = r#"
            fragment profileFields on NetplayProfile {
                id
                ratingOrdinal
                ratingUpdateCount
                wins
                losses
                dailyGlobalPlacement
                dailyRegionalPlacement
                continent
                characters {
                    id
                    character
                    gameCount
                    __typename
                }
                __typename
            }
        "#;

        let user_profile_page = r#"
            fragment userProfilePage on User {
                fbUid
                displayName
                connectCode {
                    code
                    __typename
                }
                status
                activeSubscription {
                    level
                    hasGiftSub
                    __typename
                }
                rankedNetplayProfile {
                    ...profileFields
                    __typename
                }
                netplayProfiles {
                    ...profileFields
                    season {
                    id
                    startedAt
                    endedAt
                    name
                    status
                    __typename
                    }
                    __typename
                }
                __typename
            }
        "#;

        // Combine everything into the main query
        let account_management_query = format!(r#"
            {profile_fields}
            {user_profile_page}

            query AccountManagementPageQuery($cc: String!, $uid: String!) {{
                getUser(fbUid: $uid) {{
                    ...userProfilePage
                    __typename
                }}
                getConnectCode(code: $cc) {{
                    user {{
                    ...userProfilePage
                    __typename
                    }}
                    __typename
                }}
            }}
        "#);

        let connect_code = user.connect_code;
        let variables = json!({
            "cc": connect_code,
            "uid": connect_code
        });

        let body = json!({
            "operationName": "AccountManagementPageQuery",
            "variables": variables,
            "query": account_management_query
        });

        // execute_graphql_query(api_client, account_management_query, variables);

        // let mutation = r#"
        //     mutation ($report: OnlineGameCompleteInput!) {
        //         completeOnlineGame (report: $report)
        //     }
        // "#;

        // let variables = Some(json!({
        //     "report": {
        //         "matchId": match_id,
        //         "fbUid": uid,
        //         "playKey": play_key,
        //         "endMode": end_mode,
        //     }
        // }));

        // let res = execute_graphql_query(api_client, mutation, variables, Some("completeOnlineGame"));

        // match res {
        //     Ok(value) if value == "true" => {
        //         tracing::info!(target: Log::SlippiOnline, "Successfully executed completion request")
        //     },
        //     Ok(value) => tracing::error!(target: Log::SlippiOnline, ?value, "Error executing completion request",),
        //     Err(error) => tracing::error!(target: Log::SlippiOnline, ?error, "Error executing completion request"),
        // }
    }
}

