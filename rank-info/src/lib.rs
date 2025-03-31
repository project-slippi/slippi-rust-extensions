use slippi_gg_api::APIClient;
use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};
use slippi_user::UserManager;
use slippi_user::UserInfo;
use serde::Deserialize;
use serde_json::{json, Value};

mod utils;
use utils::GetRankErrorKind;
use utils::execute_graphql_query;

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

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RankInfoAPIResponse  {
    #[serde(alias = "ratingOrdinal")]
    pub rating_ordinal: f32,

    #[serde(alias = "ratingUpdateCount")]
    pub rating_update_count: u32,

    #[serde(alias = "wins")]
    pub wins: u32,

    #[serde(alias = "losses")]
    pub losses: u32,

    #[serde(alias = "dailyGlobalPlacement", default)]
    pub daily_global_placement: Option<u8>,

    #[serde(alias = "dailyRegionalPlacement", default)]
    pub daily_regional_placement: Option<u8>,

    #[serde(alias = "continent", default)]
    pub continent: Option<String>
}

#[derive(Debug, Clone, Default)]
pub struct RankInfo {
    pub rank: u8,
    pub rating_ordinal: f32,
    pub global_placing: u8,
    pub regional_placing: u8,
    pub rating_update_count: u32,
}

#[derive(Debug)]
pub struct RankManager {
    pub api_client: APIClient,
    pub last_rank: Option<RankInfo>
}

// TODO :: Change RankInfo to RankManager and just store the api_client, remove all other fields.
//      :: Call RankManager::fetch_user_rank from the EXI device and use the user's connect code.
//      :: Send data to dolphin.

impl RankManager {
    pub fn new(api_client: APIClient) -> Self {
        Self {
            api_client: api_client,
            last_rank: None
        }
    }

    pub fn get_rank(rating_ordinal: f32, global_placing: u8, regional_placing: u8, rating_update_count: u32) -> SlippiRank {
        if rating_update_count < 5 {
            return SlippiRank::Unranked;
        }
        if rating_ordinal > 0.0 && rating_ordinal <= 765.42 {
            return SlippiRank::Bronze1;
        }
        if rating_ordinal > 765.43 && rating_ordinal <= 913.71 {
            return SlippiRank::Bronze2;
        }
        if rating_ordinal > 913.72 && rating_ordinal <= 1054.86 {
            return SlippiRank::Bronze3;
        }
        if rating_ordinal > 1054.87 && rating_ordinal <= 1188.87 {
            return SlippiRank::Silver1;
        }
        if rating_ordinal > 1188.88 && rating_ordinal <= 1315.74 {
            return SlippiRank::Silver2;
        }
        if rating_ordinal > 1315.75 && rating_ordinal <= 1435.47 {
            return SlippiRank::Silver3;
        }
        if rating_ordinal > 1435.48 && rating_ordinal <= 1548.06 {
            return SlippiRank::Gold1;
        }
        if rating_ordinal > 1548.07 && rating_ordinal <= 1653.51 {
            return SlippiRank::Gold2;
        }
        if rating_ordinal > 1653.52 && rating_ordinal <= 1751.82 {
            return SlippiRank::Gold3;
        }
        if rating_ordinal > 1751.83 && rating_ordinal <= 1842.99 {
            return SlippiRank::Platinum1;
        }
        if rating_ordinal > 1843.0 && rating_ordinal <= 1927.02 {
            return SlippiRank::Platinum2;
        }
        if rating_ordinal > 1927.03 && rating_ordinal <= 2003.91 {
            return SlippiRank::Platinum3;
        }
        if rating_ordinal > 2003.92 && rating_ordinal <= 2073.66 {
            return SlippiRank::Diamond1;
        }
        if rating_ordinal > 2073.67 && rating_ordinal <= 2136.27 {
            return SlippiRank::Diamond2;
        }
        if rating_ordinal > 2136.28 && rating_ordinal <= 2191.74 {
            return SlippiRank::Diamond3;
        }
        if rating_ordinal >= 2191.75 && global_placing > 0 && regional_placing > 0 {
            return SlippiRank::Grandmaster;
        }
        if rating_ordinal > 2191.75 && rating_ordinal <= 2274.99 {
            return SlippiRank::Master1;
        }
        if rating_ordinal > 2275.0 && rating_ordinal <= 2350.0 {
            return SlippiRank::Master2;
        }
        if rating_ordinal > 2350.0 {
            return SlippiRank::Master3;
        }
        SlippiRank::Unranked
    }

    pub fn fetch_user_rank(&mut self, connect_code: &str) -> Result<RankInfo, GetRankErrorKind> {
        let profile_fields = r#"
            fragment profileFieldsV2 on NetplayProfileV2 {
                ratingOrdinal
                ratingUpdateCount
                wins
                losses
                dailyGlobalPlacement
                dailyRegionalPlacement
                continent
            }
        "#;

        let user_profile_page = r#"
            fragment userProfilePage on User {
                rankedNetplayProfile {
                    ...profileFieldsV2
                }
            }
        "#;

        // Combine everything into the main query
        let rank_info_query = format!(r#"
            {user_profile_page}
            {profile_fields}

            query AccountManagementPageQuery($cc: String!, $uid: String!) {{
                getUser(fbUid: $uid) {{
                    ...userProfilePage
                }}
                getConnectCode(code: $cc) {{
                    user {{
                        ...userProfilePage
                    }}
                }}
            }}
        "#);

        let variables = Some(json!({
            "cc": connect_code,
            "uid": connect_code
        }));

        // TODO :: get api_client from self
        let res = execute_graphql_query(&self.api_client, &rank_info_query, variables);

        match res {
            Ok(value) => {
                tracing::info!(target: Log::SlippiOnline, ?value);
                let rank_response: Result<RankInfoAPIResponse, serde_json::Error> = serde_json::from_str(&value);
                match rank_response {
                    Ok(rank_resp) => {
                        let curr_rank = RankInfo { 
                                rank: RankManager::get_rank(
                                    rank_resp.rating_ordinal, 
                                    rank_resp.daily_global_placement.unwrap_or_default(), 
                                    rank_resp.daily_regional_placement.unwrap_or_default(),
                                    rank_resp.rating_update_count
                                ) as u8, 
                                rating_ordinal: rank_resp.rating_ordinal, 
                                global_placing: rank_resp.daily_global_placement.unwrap_or_default(), 
                                regional_placing: rank_resp.daily_regional_placement.unwrap_or_default(), 
                                rating_update_count: rank_resp.rating_update_count, 
                            };
                        // Save last response for getting rank / rating change later
                        self.last_rank = Some(curr_rank.clone());
                        Ok(curr_rank)
                    },
                    Err(_err) => Err(GetRankErrorKind::NotSuccessful("Failed to parse JSON".to_owned())),
                }
            }
            Err(err) => Err(err)
        }
    }
}

