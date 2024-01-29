use std::fs;

use lazy_static::lazy_static;
use regex::Regex;
use serde_json::Value;
use crate::util::get_appdata_file;

pub struct ConnectCode(String);
impl ConnectCode {
    pub fn is_valid(&self) -> bool {
        lazy_static! {
            static ref RE: Regex = Regex::new("^([A-Za-z0-9])+#[0-9]{1,6}$").unwrap();
        }
        RE.is_match(self.0.as_str())
    }

    pub fn as_url(&self) -> String {
        self.0.to_lowercase().replace("#", "-")
    }
}

pub fn get_connect_code() -> Option<ConnectCode> {
    if let Some(user_json_path) = get_appdata_file("Slippi Launcher/netplay/User/Slippi/user.json") {
        if user_json_path.is_file() && user_json_path.exists() {
            return fs::read_to_string(user_json_path).ok().and_then(|data| {
                match serde_json::from_str::<Value>(data.as_str()) {
                    Ok(data) => data["connectCode"].as_str().and_then(|v| Some(ConnectCode(v.into()))),
                    _ => None
                }
            });
        }
    }
    None
}