use std::sync::OnceLock;
use std::env;

mod development;
mod production;

pub(crate) static _SLP_CFG: OnceLock<SlippiConfig> = OnceLock::new();

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SlippiConfig {
    pub graphql_url: Option<String>,
}

impl SlippiConfig {
    /// Merges two configurations. Values in `other` take precedence.
    fn merge(self, other: Self) -> Self {
        Self {
            graphql_url: other.graphql_url.or(self.graphql_url),
        }
    }

    /// Default configuration values are sourced from the environment.
    fn default() -> Self {
        Self {
            graphql_url: env::var("SLIPPI_GRAPHQL_URL").ok(),
        }
    }

    #[cfg(feature = "slippi_env_development")]
    fn read_file_config(_env: &str) -> SlippiConfig {
        SlippiConfig::development()
    }

    #[cfg(feature = "slippi_env_production")]
    fn read_file_config(_env: &str) -> SlippiConfig {
        SlippiConfig::production()
    }

    /// Initializes the configuration based on a provided environment.
    fn init_config(env: &str) {
        let config: Self = Self::read_file_config(env);
        // Merge with default values
        let merged_config = Self::default().merge(config);
        tracing::warn!("SlippiRustExtensions Config {:?} Environment {}", &merged_config, SlippiConfig::get_env());
        _SLP_CFG.set(merged_config).expect("Could not initialize Config!");
    }

    /// Fetches the environment from an environment variable, defaulting to "development".
    pub const fn get_env() -> &'static str {
        match option_env!("SLIPPI_ENV") {
            None => { "development" }
            Some(env) => { env }
        }
    }

    /// Retrieves the configuration. Initializes it if it's accessed for the first time.
    pub fn get() -> Self {
        if _SLP_CFG.get().is_none() {
            Self::init_config(&Self::get_env());
        }

        _SLP_CFG.get().unwrap().clone()
    }
}
