use slippi_gg_api::APIClient;
use serde_json::{json, Value};
use crate::GetRankErrorKind;

const GRAPHQL_URL: &str = "https://gql-gateway-2-dot-slippi.uc.r.appspot.com/graphql";

/// Prepares and executes a GraphQL query.
pub(crate) fn execute_graphql_query(
    api_client: &APIClient,
    query: &str,
    variables: Option<Value>,
    field: Option<&str>,
) -> Result<String, GetRankErrorKind> {
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

    // Parse the response JSON
    let response_json: Value =
        serde_json::from_str(&response.into_string().unwrap_or_default()).map_err(GetRankErrorKind::JSON)?;

    // Check for GraphQL errors
    if let Some(errors) = response_json.get("errors") {
        if errors.is_array() && !errors.as_array().unwrap().is_empty() {
            let error_message = serde_json::to_string_pretty(errors).unwrap();
            return Err(GetRankErrorKind::GraphQL(error_message));
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
        Err(GetRankErrorKind::GraphQL(
            "No 'data' field in the GraphQL response.".to_string(),
        ))
    }
}
