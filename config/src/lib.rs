use std::sync::OnceLock;
use std::env;
use std::fs;
use std::path::PathBuf;

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

    /// Initializes the configuration based on a provided environment.
    fn init_config(env: &str) {
        let path = PathBuf::from(file!())
            .parent()
            .expect("Failed to get parent directory")
            .join(format!("../envs/{}.toml", env));

        // Read the file content
        let contents = fs::read_to_string(&path).expect("Unable to read the file");

        // Deserialize the TOML contents into the struct
        let file_config: Self = toml::from_str(&contents).expect("Failed to parse TOML");

        // Merge with default values
        let merged_config = Self::default().merge(file_config);

        _SLP_CFG.set(merged_config).expect("Could not initialize Config!");
    }

    /// Fetches the environment from an environment variable, defaulting to "development".
    fn get_env() -> String {
        env::var("SLIPPI_ENV").unwrap_or_else(|_| String::from("development"))
    }

    /// Retrieves the configuration. Initializes it if it's accessed for the first time.
    pub fn get() -> Self {
        if _SLP_CFG.get().is_none() {
            Self::init_config(&Self::get_env());
        }

        _SLP_CFG.get().unwrap().clone()
    }
}
