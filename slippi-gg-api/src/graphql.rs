use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use super::APIClient;

/// Various errors that can happen during a GraphQL request.
#[derive(Debug)]
pub enum GraphQLError {
    FailedErrorFormatting,
    MissingResponseField,
    MissingResponseData,
    Request(ureq::Error),
    IO(std::io::Error),
    InvalidResponseType(serde_json::Error),
    InvalidResponseJSON(serde_json::Error),
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
    body: HashMap<&'static str, Value>
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
            body
        }
    }

    /// Sets optional `variables` for the GraphQL payload.
    pub fn variables(mut self, variables: Value) -> Self {
        self.body.insert("variables", variables);
        self
    }

    /// Sets an optional key that the response handler should use as its
    /// return type. If this is not configured, the response handler will
    /// use the entire `data` payload for deserialization.
    pub fn data_field<Key>(mut self, key: Key) -> Self
    where
        Key: Into<Cow<'static, str>>,
    {
        self.response_field = Some(key.into());
        self
    }

    /// Consumes and sends the request, deserializing the response and yielding
    /// any errors in the process.
    pub fn send<'a, T>(self) -> Result<T, GraphQLError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response_body = self.client
            .post(self.endpoint.as_ref())
            .send_json(&self.body)
            .map_err(GraphQLError::Request)?
            .into_string()
            .map_err(GraphQLError::IO)?;

        // We always go through `Value` first in order to check any
        // potential errors and remove anything the caller doesn't need.
        let mut response: Value = serde_json::from_str(&response_body).map_err(|error| {
            tracing::error!(?error, "Failed to deserialize GraphQL response");
            GraphQLError::InvalidResponseType(error)
        })?;

        // Errors will always be in the `errors` slot, so check that first.
        if let Some(errors) = response.get("errors") {
            if errors.is_array() && !errors.as_array().unwrap().is_empty() {
                match serde_json::to_string_pretty(errors) {
                    Ok(error_messages) => {
                        return Err(GraphQLError::Server(error_messages));
                    },

                    Err(error) => {
                        tracing::error!(?error, "Failed to pretty-format error string");
                        return Err(GraphQLError::FailedErrorFormatting);
                    }
                }
            }
        }

        // Now attempt to extract the response payload. If we have it, then we'll attempt
        // to deserialize it to the expected response type.
        let mut data = response
            .get_mut("data")
            .ok_or(GraphQLError::MissingResponseData)?
            .take();

        // Search further in the payload if we've set a key to use.
        if let Some(field) = self.response_field {
            data = data
                .get_mut(field.as_ref())
                .ok_or(GraphQLError::MissingResponseField)?
                .take();
        }

        serde_json::from_value(data)
            .map_err(GraphQLError::InvalidResponseJSON)
    }
}
