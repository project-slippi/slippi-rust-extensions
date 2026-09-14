//! Wire types for the matchmaking WebSocket. These mirror the service's
//! protocol package field for field.

use serde::{Deserialize, Serialize};

/// The identity the client claims. The service verifies it.
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub uid: String,
    #[serde(rename = "playKey")]
    pub play_key: String,
    #[serde(rename = "connectCode")]
    pub connect_code: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

/// Which queue to enter and, for direct and teams, the code to match on. The
/// code is sent as the raw Shift-JIS bytes the game provides; the service
/// decodes them.
#[derive(Debug, Serialize)]
pub struct Search {
    pub mode: u8,
    #[serde(rename = "connectCode")]
    pub connect_code: Vec<u8>,
}

/// An address the player may be reachable at for netplay.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub ip: String,
    pub port: u16,
    /// "host" for a local interface, "srflx" for a STUN-discovered public
    /// address, "observed" for what the service saw the client connect from.
    #[serde(rename = "type")]
    pub kind: String,
}

/// Overrides the service honors only when it runs with authentication
/// disabled. Used by tests and local development.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DebugOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mmr: Option<f64>,
}

/// First message on a fresh connection.
#[derive(Debug, Serialize)]
pub struct Join {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub user: UserInfo,
    pub search: Search,
    #[serde(rename = "appVersion")]
    pub app_version: String,
    /// The UDP port opponents should send to. This is the STUN-mapped port
    /// when known, otherwise the local port.
    pub port: u16,
    #[serde(rename = "ipAddressLan")]
    pub lan_addr: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    #[serde(rename = "natType", skip_serializing_if = "String::is_empty")]
    pub nat_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugOverrides>,
}

/// First message when reattaching to an existing ticket.
#[derive(Debug, Serialize)]
pub struct Resume {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "ticketId")]
    pub ticket_id: String,
    pub token: String,
}

/// A message with only a type, such as cancel or ping.
#[derive(Debug, Serialize)]
pub struct Simple {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

/// Read first to learn what a server message is.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct Joined {
    #[serde(rename = "ticketId")]
    pub ticket_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct Status {
    #[serde(rename = "secondsInQueue")]
    pub seconds_in_queue: f64,
}

#[derive(Debug, Deserialize)]
pub struct ErrorMessage {
    pub error: String,
    #[serde(rename = "latestVersion", default)]
    pub latest_version: String,
}

/// Only the fields needed to log a match; the full JSON is handed to Dolphin.
#[derive(Debug, Deserialize)]
pub struct MatchSummary {
    #[serde(rename = "matchId")]
    pub match_id: String,
    #[serde(rename = "isHost", default)]
    pub is_host: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_serializes_with_service_field_names() {
        let join = Join {
            kind: "join",
            user: UserInfo {
                uid: "u".into(),
                play_key: "k".into(),
                connect_code: "ME#1".into(),
                display_name: "Me".into(),
            },
            search: Search {
                mode: 2,
                connect_code: vec![0x82, 0x60],
            },
            app_version: "3.4.0".into(),
            port: 41000,
            lan_addr: "192.168.1.5:41000".into(),
            candidates: vec![Candidate {
                ip: "1.2.3.4".into(),
                port: 41000,
                kind: "srflx".into(),
            }],
            nat_type: "cone".into(),
            debug: None,
        };

        let value: serde_json::Value = serde_json::from_str(&serde_json::to_string(&join).unwrap()).unwrap();
        assert_eq!(value["type"], "join");
        assert_eq!(value["user"]["playKey"], "k");
        assert_eq!(value["search"]["connectCode"], serde_json::json!([130, 96]));
        assert_eq!(value["ipAddressLan"], "192.168.1.5:41000");
        assert_eq!(value["candidates"][0]["type"], "srflx");
        assert_eq!(value["natType"], "cone");
        assert!(value.get("debug").is_none());
    }

    #[test]
    fn empty_optional_fields_are_omitted() {
        let join = Join {
            kind: "join",
            user: UserInfo {
                uid: "u".into(),
                play_key: "k".into(),
                connect_code: "ME#1".into(),
                display_name: "Me".into(),
            },
            search: Search {
                mode: 1,
                connect_code: vec![],
            },
            app_version: "3.4.0".into(),
            port: 41000,
            lan_addr: String::new(),
            candidates: vec![],
            nat_type: String::new(),
            debug: None,
        };
        let text = serde_json::to_string(&join).unwrap();
        assert!(!text.contains("candidates"));
        assert!(!text.contains("natType"));
    }
}
