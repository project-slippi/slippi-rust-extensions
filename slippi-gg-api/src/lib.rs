use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::ops::{Deref, DerefMut};
use std::time::Duration;

use ureq::{Agent, AgentBuilder, Resolver};

mod graphql;
pub use graphql::{GraphQLBuilder, GraphQLError};

/// Re-export `ureq::Error` for simplicity.
pub type Error = ureq::Error;

/// A DNS resolver that only accepts IPV4 connections.
struct Ipv4Resolver;

impl Resolver for Ipv4Resolver {
    /// Forces IPV4 addresses only.
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        ToSocketAddrs::to_socket_addrs(netloc).map(|iter| {
            let vec = iter.filter(|s| s.is_ipv4()).collect::<Vec<SocketAddr>>();

            if vec.is_empty() {
                tracing::warn!("Failed to get any IPV4 addresses. Does the DNS server support it?");
            }

            vec
        })
    }
}

/// Default timeout that we use on client types. Extracted
/// so that the GraphQLBuilder can also call it.
pub(crate) fn default_timeout() -> Duration {
    Duration::from_millis(5000)
}

/// A wrapper type that simply dereferences to a `ureq::Agent`.
///
/// It's extracted purely for ease of debugging, and for segmenting
/// some initial setup code that would just be cumbersome to do in the
/// core EXI device initialization block.
///
/// Anything that can be called on a `ureq::Agent` can be called on
/// this type. You can also clone this with little cost, and pass it freely
/// to other threads, as it manages itself under the hood with `Arc`.
#[derive(Clone, Debug)]
pub struct APIClient(Agent);

impl APIClient {
    /// Creates and initializes a new APIClient.
    ///
    /// The returned client will only resolve to IPV4 addresses at the moment
    /// due to upstream issues with GCP flex instances and IPV6.
    pub fn new(slippi_semver: &str) -> Self {
        let _build = "";

        #[cfg(feature = "mainline")]
        let _build = "mainline";

        #[cfg(feature = "ishiiruka")]
        let _build = "ishiiruka";

        #[cfg(feature = "playback")]
        let _build = "playback";

        // We set `max_idle_connections` to `5` to mimic how CURL was configured in
        // the old C++ logic. This gets cloned and passed down into modules so that
        // the underlying connection pool is shared.
        let http_client = AgentBuilder::new()
            .resolver(Ipv4Resolver)
            .max_idle_connections(5)
            .timeout(default_timeout())
            .user_agent(&format!("SlippiDolphin/{} ({}) (Rust)", _build, slippi_semver))
            .build();

        Self(http_client)
    }

    /// Returns a type that can be used to construct GraphQL requests.
    pub fn graphql<Query>(&self, query: Query) -> GraphQLBuilder
    where
        Query: Into<String>,
    {
        GraphQLBuilder::new(self.clone(), query.into())
    }
}

impl Deref for APIClient {
    type Target = Agent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for APIClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
