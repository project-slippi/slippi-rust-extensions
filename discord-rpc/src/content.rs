//! Turns the presence thread's state into the strings shown in a Discord
//! update. Pure rendering; `PresenceRunner` owns the state these read.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::melee;
use crate::parser::GameStartInfo;

/// Everything needed to render one presence update.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PresenceContent {
    pub details: String,
    pub state: String,
    pub large_image: String,
    pub large_text: String,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    pub start_timestamp: Option<i64>,
    /// `(current, max)` party size, rendered by Discord as "(current of max)".
    pub party: Option<(u32, u32)>,
}

/// State of the game currently being played, built up from replay events.
#[derive(Debug, Default)]
pub(crate) struct GameState {
    pub info: GameStartInfo,
    pub stocks: [u8; 4],
    pub started_at: i64,
    pub ended: bool,
}

/// Wins per port for the current set; reset whenever the match ID changes.
#[derive(Debug, Default)]
pub(crate) struct SetScore {
    pub match_id: String,
    pub wins: [u8; 4],
}

/// Maps a rank index to its display name, which also derives the asset key.
pub(crate) fn rank_name(rank: i8) -> Option<&'static str> {
    Some(match rank {
        1 => "Bronze 1",
        2 => "Bronze 2",
        3 => "Bronze 3",
        4 => "Silver 1",
        5 => "Silver 2",
        6 => "Silver 3",
        7 => "Gold 1",
        8 => "Gold 2",
        9 => "Gold 3",
        10 => "Platinum 1",
        11 => "Platinum 2",
        12 => "Platinum 3",
        13 => "Diamond 1",
        14 => "Diamond 2",
        15 => "Diamond 3",
        16 => "Master 1",
        17 => "Master 2",
        18 => "Master 3",
        19 => "Grandmaster",
        _ => return None,
    })
}

/// Derives the Discord rank-badge asset key from a rank display name,
/// e.g. "Diamond 1" -> "diamond_1".
pub(crate) fn rank_asset_key(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// The `details` line for an in-game activity: the mode, game number within
/// the set, and (once a game ends) the running set score.
pub(crate) fn game_details(game: &GameState, set_score: &SetScore) -> String {
    let mode = match &game.info.match_id {
        id if id.contains("mode.ranked") => "Ranked",
        id if id.contains("mode.unranked") => "Unranked",
        id if id.contains("mode.direct") => "Direct",
        id if id.contains("mode.teams") => "Teams",
        _ => "Online",
    };

    let mut details = mode.to_string();

    if game.info.game_number > 0 {
        details.push_str(&format!(" - Game {}", game.info.game_number));
    }

    if game.info.tiebreaker_number > 0 {
        details.push_str(" (tiebreak)");
    }

    if game.ended {
        let wins: Vec<String> = game
            .info
            .players
            .iter()
            .map(|p| set_score.wins[p.port as usize].to_string())
            .collect();

        details.push_str(&format!(" - Set {}", wins.join("-")));
    }

    details
}

/// The `state` line for an in-game activity: the two players and their live
/// stock counts, or a game-over summary once the game ends.
pub(crate) fn game_state_line(game: &GameState) -> String {
    let players = &game.info.players;

    if players.len() != 2 {
        return "In game".to_string();
    }

    let name = |i: usize| -> String {
        if !players[i].display_name.is_empty() {
            players[i].display_name.clone()
        } else {
            melee::character_name(players[i].character_id).unwrap_or("Player").to_string()
        }
    };

    if game.ended {
        return format!("{} vs {} - Game over", name(0), name(1));
    }

    let stocks = |i: usize| game.stocks[players[i].port as usize];

    format!("{} {} - {} {}", name(0), stocks(0), stocks(1), name(1))
}

/// Current Unix time in seconds.
pub(crate) fn unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
