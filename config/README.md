# SlippiConfig Module

The `SlippiConfig` module provides functionalities for handling configurations specific to Slippi. It sources configuration values either from a TOML file or environment variables.

## Features

- **Lazy Initialization**: Configurations are loaded the first time they are accessed.
- **Environment Specific**: Load configuration based on the current environment (e.g., development, production).
- **Fallbacks**: In the absence of a configuration value in the TOML file, environment variables are used as defaults.

## Structure

### SlippiConfig Struct

This structure holds the Slippi configuration values. Currently, it supports:

- `graphql_url`: An optional string that represents the URL for GraphQL used for reporting matches.

### Initialization and Retrieval

- `SlippiConfig::get()`: Retrieve the configuration. If it's the first access, it initializes the configuration based on the provided environment.

## Usage

Ensure you have a `.toml` configuration file under the `envs/` directory named after the environment (e.g., `development.toml`).

To use the configuration:

```rust
let config = SlippiConfig::get();
println!("{:?}", config.graphql_url);
```
