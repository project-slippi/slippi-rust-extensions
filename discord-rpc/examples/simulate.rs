//! Replays one or more `.slp` files through the Discord presence pipeline
//! so the integration can be tested without building or running Dolphin.
//! Passing multiple files from the same set exercises set-score tracking.
//!
//! Usage:
//!     cargo run -p slippi-discord-rpc --example simulate -- <replay.slp>... [speed]
//!
//! `speed` is a playback multiplier (default 8). Have Discord running and
//! watch your profile while this runs.

use std::time::Duration;

use slippi_discord_rpc::{DiscordHandler, raw_replay_stream};

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let mut args: Vec<String> = std::env::args().skip(1).collect();

    // A trailing integer argument is the speed multiplier.
    let speed: u32 = match args.last().and_then(|arg| arg.parse().ok()) {
        Some(speed) => {
            args.pop();
            speed
        },
        None => 8,
    };

    assert!(!args.is_empty(), "usage: simulate <replay.slp>... [speed]");

    let handler = DiscordHandler::new(None).expect("could not start the presence handler");

    // Feed each stream in chunks, pacing by approximate frame density so
    // the presence updates roll in like a real (sped up) game. A frame of
    // replay data is roughly 400 bytes, and Melee runs at 60 frames a
    // second.
    let bytes_per_second = 400 * 60 * speed as usize;
    let chunk_size = bytes_per_second / 20;

    for (i, path) in args.iter().enumerate() {
        let file = std::fs::read(path).expect("could not read replay file");
        let stream = raw_replay_stream(&file).expect("not a valid .slp replay");

        println!(
            "Game {}: {} ({} bytes of event data) at {speed}x speed",
            i + 1,
            path,
            stream.len()
        );

        for chunk in stream.chunks(chunk_size) {
            handler.push_replay_data(chunk);
            std::thread::sleep(Duration::from_millis(50));
        }

        // Linger on the result like the post-game screen would.
        std::thread::sleep(Duration::from_secs(6));
    }

    println!("Set complete; holding presence for 10 seconds...");
    std::thread::sleep(Duration::from_secs(10));

    // Dropping the handler shuts the thread down and clears the activity.
    drop(handler);

    println!("Done.");
}
