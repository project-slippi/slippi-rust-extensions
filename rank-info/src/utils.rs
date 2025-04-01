use slippi_gg_api::APIClient;
use serde_json::{json, Value};

const GRAPHQL_URL: &str = "https://gql-gateway-2-dot-slippi.uc.r.appspot.com/graphql";

/// The true inner error, minus any metadata.
#[derive(Debug)]
pub enum GetRankErrorKind {
    Net(slippi_gg_api::Error),
    JSON(serde_json::Error),
    GraphQL(String),
    NotSuccessful(String),
}

pub(crate) fn execute_rank_query(
    api_client: &APIClient,
    connect_code: &str,
) -> Result<String, GetRankErrorKind> {
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

    let query = format!(r#"
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
        if let Some(get_connect_code) = data.get("getConnectCode") {
            if let Some(user) = get_connect_code.get("user") {
                if let Some(profile) = user.get("rankedNetplayProfile") {
                    return Ok(profile.to_string());
                }
                return Err(GetRankErrorKind::NotSuccessful("rankedNetplayProfile".into()));
            }
            return Err(GetRankErrorKind::NotSuccessful("user".into()));
        }
        return Err(GetRankErrorKind::NotSuccessful("getConnectCode".into()));
    } else {
        Err(GetRankErrorKind::GraphQL(
            "No 'data' field in the GraphQL response.".to_string(),
        ))
    }
}
