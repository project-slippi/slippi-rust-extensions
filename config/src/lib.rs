
use std::sync::OnceLock;
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Configuration {
    pub graphql_url: String,
}


pub(crate) static _SLP_CFG: OnceLock<Configuration> = OnceLock::new();


pub struct SlippiConfig;

impl SlippiConfig {

    fn init_config(env: &String) {
        // Read the YAML file
        let config_path = format!("config/{}.toml", env);
        let path = Path::new(&config_path);
        let mut file = File::open(&path).expect("Unable to open file");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("Unable to read the file");

        // Deserialize the YAML contents into the struct
        let config: Configuration = serde_yaml::from_str(&contents).unwrap();
        println!("{:?}", config);

        _SLP_CFG.set(config).expect("Could not initialize Configuration!");
    }

    fn get_env() -> String {
        env::var("SLIPPI_ENV").unwrap_or(String::from("development"))
    }

    pub fn get() -> Configuration
    {
        if _SLP_CFG.get().is_none() {
            SlippiConfig::init_config(&SlippiConfig::get_env());
        }

        _SLP_CFG.get().unwrap().clone()
    }
}
