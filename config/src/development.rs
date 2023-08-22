use crate::SlippiConfig;

impl SlippiConfig {
    pub fn development() -> Self {
        Self {
            graphql_url: Some(String::from("https://gql-gateway-dev-dot-slippi.uc.r.appspot.com/graphql")),
        }
    }
}
