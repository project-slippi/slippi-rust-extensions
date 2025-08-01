use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;
use thiserror::Error;

use dolphin_integrations::Log;

use super::APIClient;

/// Various errors that can happen during a GraphQL request.
#[derive(Debug, Error)]
pub enum GraphQLError {
    #[error("Expected {0} data key, but returned payload has none.")]
    MissingResponseField(String),

    #[error("Expected data key, but returned payload has none.")]
    MissingResponseData,

    #[error(transparent)]
    Request(ureq::Error),

    #[error(transparent)]
    IO(std::io::Error),

    #[error(transparent)]
    InvalidResponseType(serde_json::Error),

    #[error(transparent)]
    InvalidResponseJSON(serde_json::Error),

    #[error("GraphQL call returned errors: {0}")]
    Server(String),
}

/// A builder pattern that makes constructing and parsing GraphQL
/// responses simpler.
///
/// You generally shouldn't create this type yourself; call `.graphql()`
/// on an `APIClient` instance to receive one for use.
#[derive(Debug)]
pub struct GraphQLBuilder {
    client: APIClient,
    endpoint: Cow<'static, str>,
    response_field: Option<Cow<'static, str>>,
    body: HashMap<&'static str, Value>,
}

impl GraphQLBuilder {
    /// Creates and returns a new GraphQLBuilder type.
    pub fn new(client: APIClient, query: String) -> Self {
        let mut body = HashMap::new();
        body.insert("query", Value::String(query));

        Self {
            client,
            endpoint: Cow::Borrowed("https://internal.slippi.gg/graphql"),
            response_field: None,
            body,
        }
    }

    /// Sets optional `variables` for the GraphQL payload.
    ///
    /// In the future, this might be widened to accept any type
    /// that implements `serde::Serialize`. At the moment all our
    /// calls work on built `Value` types using the `json!()` macro
    /// anyway so there's no need to complicate the builder chain with it.
    pub fn variables(mut self, variables: Value) -> Self {
        self.body.insert("variables", variables);
        self
    }

    /// Sets an optional key that the response handler should use as its
    /// return type. If this is not configured, the response handler will
    /// use the entire `data` payload for deserialization.
    pub fn data_field<Pointer>(mut self, pointer: Pointer) -> Self
    where
        Pointer: Into<Cow<'static, str>>,
    {
        self.response_field = Some(pointer.into());
        self
    }

    /// Consumes and sends the request, deserializing the response and yielding
    /// any errors in the process.
    pub fn send<T>(self) -> Result<T, GraphQLError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .client
            .post(self.endpoint.as_ref())
            .send_json(&self.body)
            .map_err(GraphQLError::Request)?
            .into_string()
            .map_err(GraphQLError::IO)?;

        parse(&self, &response).inspect_err(|error| match error {
            // This is a fully parsed error from the server, so we don't
            // need to keep the response body around for debugging.
            GraphQLError::Server(_) => {},

            // For non-parsable error situations, we want to go ahead and
            // dump the response body to make debugging easier.
            _ => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    "GraphQL response body: {}",
                    response
                );
            },
        })
    }
}

/// Attempts to parse a returned response body.
///
/// This is mostly separated to provide a more concise `GraphQLBuilder::send`
/// method with regards to some specific logging we want to do.
fn parse<T>(request: &GraphQLBuilder, response_body: &str) -> Result<T, GraphQLError>
where
    T: serde::de::DeserializeOwned,
{
    // We always go through `Value` first in order to check any
    // potential errors and remove anything the caller doesn't need.
    let mut response: Value = serde_json::from_str(response_body).map_err(|error| {
        tracing::error!(target: Log::SlippiOnline, ?error, "Failed to deserialize GraphQL response");
        GraphQLError::InvalidResponseType(error)
    })?;

    // Errors will always be in the `errors` slot, so check that first.
    if let Some(errors) = response.get("errors") {
        if errors.is_array() && !errors.as_array().unwrap().is_empty() {
            // In the event that pretty printing somehow fails, just fall back
            // to the `Value` debug impl. It'll communicate well enough what
            // happened and is a rare edge case anyway.
            let messages = serde_json::to_string_pretty(&errors).map_err(|error| {
                tracing::error!(target: Log::SlippiOnline, ?error, "Failed to pretty-format error string");
                GraphQLError::Server(format!("{:?}", errors))
            })?;

            return Err(GraphQLError::Server(messages));
        }
    }

    // Now attempt to extract the response payload. If we have it, then we'll attempt
    // to deserialize it to the expected response type.
    let data = if let Some(path) = &request.response_field {
        response
            .pointer_mut(path.as_ref())
            .ok_or_else(|| GraphQLError::MissingResponseField(path.to_string()))?
            .take()
    } else {
        response.get_mut("data").ok_or(GraphQLError::MissingResponseData)?.take()
    };

    serde_json::from_value(data).map_err(GraphQLError::InvalidResponseJSON)
}
