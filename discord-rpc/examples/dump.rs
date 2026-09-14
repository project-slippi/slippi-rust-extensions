//! Parses a `.slp` file and dumps the events the presence pipeline sees.
//! Debug aid only.

use slippi_discord_rpc::{EventParser, SlpEvent, raw_replay_stream};

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <replay.slp>");
    let file = std::fs::read(&path).expect("could not read replay file");
    let stream = raw_replay_stream(&file).expect("not a valid .slp replay");

    let mut parser = EventParser::new();
    let mut post_frames_seen = 0;

    parser.push(stream, |event| match event {
        SlpEvent::GameStart(info) => println!("{info:#?}"),
        SlpEvent::PostFrame { player, stocks, percent } => {
            if post_frames_seen < 8 {
                println!("PostFrame player={player} stocks={stocks} percent={percent}");
            }
            post_frames_seen += 1;
        },
        SlpEvent::GameEnd { .. } => println!("{event:?} (after {post_frames_seen} post-frames)"),
    });
}
