//! Walks the Discord presence through the menu / matchmaking flow by pushing
//! `update_matchmaking_state` snapshots, the same way Slippi's C++ side does.
//! No Dolphin or emulated memory needed.
//!
//! Usage:
//!     cargo run -p slippi-discord-rpc --example simulate_queue
//!
//! Have Discord running and watch your profile move through the matchmaking
//! states.

use std::time::Duration;

use slippi_discord_rpc::DiscordHandler;

// SlippiMatchmaking::ProcessState
const IDLE: u8 = 0;
const MATCHMAKING: u8 = 2;
const OPPONENT_CONNECTING: u8 = 3;
const CONNECTION_SUCCESS: u8 = 4;

// SlippiMatchmaking::OnlinePlayMode
const RANKED: u8 = 0;

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();

    let handler = DiscordHandler::new(None).expect("could not start the presence handler");

    let step = |label: &str, process: u8, mode: u8, opponent: Option<&str>, rank: i8| {
        println!("--- {label}");
        handler.update_matchmaking_state(process, mode, opponent.map(str::to_string), rank);
        std::thread::sleep(Duration::from_secs(6));
    };

    step("In menus", IDLE, RANKED, None, -1);
    step("In queue - Ranked", MATCHMAKING, RANKED, None, -1);
    step(
        "Opponent found - Mango (Diamond 1)",
        OPPONENT_CONNECTING,
        RANKED,
        Some("Mango"),
        13,
    );
    step("Starting match", CONNECTION_SUCCESS, RANKED, Some("Mango"), 13);

    println!("--- Done; clearing presence");
    drop(handler);
}
