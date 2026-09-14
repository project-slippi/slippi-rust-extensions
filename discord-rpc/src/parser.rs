//! An incremental parser for the Slippi replay event stream.
//!
//! This is the same byte stream that gets written to `.slp` files: the game
//! writes replay events over EXI, Dolphin forwards each payload to
//! `slprs_exi_device_reporter_push_replay_data`, and we tap that flow here.
//!
//! Only the handful of events (and fields) needed for rich presence are
//! parsed; everything else is skipped via the payload size table. Offsets
//! reference the spec at
//! https://github.com/project-slippi/slippi-wiki/blob/master/SPEC.md and
//! include the command byte (i.e. offset 0x0 is the command itself).

use dolphin_integrations::Log;

const CMD_EVENT_PAYLOADS: u8 = 0x35;
const CMD_GAME_START: u8 = 0x36;
const CMD_POST_FRAME: u8 = 0x38;
const CMD_GAME_END: u8 = 0x39;

/// Extracts the raw event stream from a `.slp` file, or `None` if the data is
/// too short or isn't a `.slp`. The `.slp` container is UBJSON whose `raw`
/// element holds the same byte stream the EXI device feeds us live, so a saved
/// game can be replayed through `EventParser`. See the crate examples.
pub fn raw_replay_stream(slp_file: &[u8]) -> Option<&[u8]> {
    // `{U\x03raw[$U#l`, then a u32 big-endian length, then the stream.
    const HEADER: &[u8] = b"{U\x03raw[$U#l";

    if slp_file.len() < 15 || &slp_file[..11] != HEADER {
        return None;
    }

    let length = u32::from_be_bytes(slp_file[11..15].try_into().ok()?) as usize;
    slp_file.get(15..15 + length)
}

/// Per-player information pulled from the `Game Start` event.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfo {
    pub port: u8,
    pub character_id: u8,
    pub costume_id: u8,
    pub starting_stocks: u8,
    pub display_name: String,
    pub connect_code: String,
}

/// Information pulled from the `Game Start` event (replay versions >= 3.14
/// fill in the match fields; older versions leave them empty/zero).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameStartInfo {
    pub stage_id: u16,
    pub players: Vec<PlayerInfo>,
    pub match_id: String,
    pub game_number: u32,
    pub tiebreaker_number: u32,
}

/// Events that consumers of the stream care about.
#[derive(Clone, Debug, PartialEq)]
pub enum SlpEvent {
    GameStart(GameStartInfo),

    /// Emitted once per player per frame; carries the live stock count
    /// and damage so a consumer can track score changes.
    PostFrame {
        player: u8,
        stocks: u8,
        percent: f32,
    },

    /// `end_method`: 1 = TIME!, 2 = GAME!, 7 = No Contest.
    /// `placements[port]` is 0 for the winner (-1 when not applicable).
    GameEnd {
        end_method: u8,
        lras_initiator: i8,
        placements: [i8; 4],
    },
}

/// Accumulates raw stream bytes and emits `SlpEvent`s as complete events
/// become available. Feed it data with `push`; chunk boundaries don't need
/// to align with event boundaries.
#[derive(Debug)]
pub struct EventParser {
    buffer: Vec<u8>,
    payload_sizes: [u16; 256],
}

impl EventParser {
    /// Creates an empty parser. The payload-size table is learned from the
    /// first `Event Payloads` command in the stream.
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            payload_sizes: [0; 256],
        }
    }

    /// Appends a chunk of the replay stream and invokes `on_event` once for
    /// each complete event. Chunk boundaries need not align with event
    /// boundaries.
    pub fn push(&mut self, data: &[u8], mut on_event: impl FnMut(SlpEvent)) {
        // A fresh Event Payloads command starts a new game, and Dolphin
        // forwards each payload as a whole chunk, so leftover buffered bytes
        // when one arrives mean the previous game's stream was cut short.
        // Resync onto the new game rather than wait for a truncated event.
        if data.first() == Some(&CMD_EVENT_PAYLOADS) && !self.buffer.is_empty() {
            tracing::info!(
                target: Log::DiscordRpc,
                discarded = self.buffer.len(),
                "New game started mid-event, discarding truncated stream data"
            );
            self.buffer.clear();
        }

        self.buffer.extend_from_slice(data);

        loop {
            let Some(&command) = self.buffer.first() else {
                return;
            };

            let event_len = if command == CMD_EVENT_PAYLOADS {
                // Need the length byte; a declared length of 0 is malformed,
                // so drop the byte and resync rather than looping on it.
                match self.buffer.get(1) {
                    None => return,
                    Some(0) => {
                        self.buffer.remove(0);
                        continue;
                    },
                    Some(&len) => 1 + len as usize,
                }
            } else {
                match self.payload_sizes[command as usize] {
                    // An unknown command means we've lost sync with the
                    // stream (or never saw an Event Payloads command). Drop
                    // the buffer rather than spinning forever.
                    0 => {
                        tracing::warn!(target: Log::DiscordRpc, command, "Unknown replay command, resetting parser");
                        self.buffer.clear();
                        return;
                    },
                    size => 1 + size as usize,
                }
            };

            if self.buffer.len() < event_len {
                return;
            }

            let event: Vec<u8> = self.buffer.drain(..event_len).collect();

            match command {
                CMD_EVENT_PAYLOADS => self.read_payload_sizes_table(&event),
                CMD_GAME_START => {
                    if let Some(info) = parse_game_start(&event) {
                        on_event(SlpEvent::GameStart(info));
                    }
                },
                CMD_POST_FRAME => {
                    if let Some(event) = parse_post_frame(&event) {
                        on_event(event);
                    }
                },
                CMD_GAME_END => {
                    if let Some(event) = parse_game_end(&event) {
                        on_event(event);
                    }
                },
                _ => {},
            }
        }
    }

    /// The Event Payloads command declares the byte size of every other
    /// command in the stream; it's always the first event of a game.
    fn read_payload_sizes_table(&mut self, event: &[u8]) {
        self.payload_sizes = [0; 256];

        // `get(2..)` rather than `event[2..]`: a degenerate (0- or 1-byte
        // payload) declaration would otherwise panic, and a panic on this
        // thread aborts the whole process. A short event just yields an empty
        // table, which self-heals via the unknown-command reset in `push`.
        for entry in event.get(2..).unwrap_or_default().chunks_exact(3) {
            let command = entry[0] as usize;
            self.payload_sizes[command] = u16::from_be_bytes([entry[1], entry[2]]);
        }
    }
}

impl Default for EventParser {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_game_start(event: &[u8]) -> Option<GameStartInfo> {
    let stage_id = read_u16(event, 0x13)?;

    let mut players = Vec::new();

    for port in 0u8..4 {
        let i = port as usize;

        // Player type: 0 = human, 1 = CPU, 2 = demo, 3 = empty.
        let player_type = read_u8(event, 0x66 + 0x24 * i)?;
        if player_type > 1 {
            continue;
        }

        players.push(PlayerInfo {
            port,
            character_id: read_u8(event, 0x65 + 0x24 * i)?,
            starting_stocks: read_u8(event, 0x67 + 0x24 * i)?,
            costume_id: read_u8(event, 0x68 + 0x24 * i)?,
            // Added in replay version 3.9.0; reads resolve to empty
            // strings on older versions.
            display_name: read_shift_jis_string(event, 0x1A5 + 0x1F * i, 31),
            connect_code: read_shift_jis_string(event, 0x221 + 0xA * i, 10),
        });
    }

    Some(GameStartInfo {
        stage_id,
        players,
        // Added in replay version 3.14.0.
        match_id: read_shift_jis_string(event, 0x2BE, 51),
        game_number: read_u32(event, 0x2F1).unwrap_or(0),
        tiebreaker_number: read_u32(event, 0x2F5).unwrap_or(0),
    })
}

fn parse_post_frame(event: &[u8]) -> Option<SlpEvent> {
    // Ignore follower entities (Ice Climbers' Nana).
    if read_u8(event, 0x6)? != 0 {
        return None;
    }

    Some(SlpEvent::PostFrame {
        player: read_u8(event, 0x5)?,
        stocks: read_u8(event, 0x21)?,
        percent: f32::from_bits(read_u32(event, 0x16)?),
    })
}

fn parse_game_end(event: &[u8]) -> Option<SlpEvent> {
    let mut placements = [-1i8; 4];

    // Placements were added in replay version 3.13.0.
    for (i, placement) in placements.iter_mut().enumerate() {
        if let Some(value) = read_u8(event, 0x3 + i) {
            *placement = value as i8;
        }
    }

    Some(SlpEvent::GameEnd {
        end_method: read_u8(event, 0x1)?,
        lras_initiator: read_u8(event, 0x2).map(|v| v as i8).unwrap_or(-1),
        placements,
    })
}

fn read_u8(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied()
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(offset..offset + 2)?.try_into().ok()?))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(offset..offset + 4)?.try_into().ok()?))
}

/// Reads a null-terminated Shift-JIS string field.
fn read_shift_jis_string(data: &[u8], offset: usize, max_len: usize) -> String {
    match data.get(offset..offset + max_len) {
        Some(field) => crate::text::decode_shift_jis(field),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a miniature replay stream: an Event Payloads declaration, a
    /// Game Start, one Post Frame per player and a Game End.
    fn test_stream() -> Vec<u8> {
        let mut stream = vec![
            CMD_EVENT_PAYLOADS,
            10,
            CMD_GAME_START,
            0x02,
            0xF8,
            CMD_POST_FRAME,
            0x00,
            0x50,
            CMD_GAME_END,
            0x00,
            0x06,
        ];

        let mut game_start = vec![0u8; 0x2F8 + 1];
        game_start[0] = CMD_GAME_START;
        game_start[0x13..0x15].copy_from_slice(&31u16.to_be_bytes());

        for (i, character) in [(0usize, 2u8), (1usize, 20u8)] {
            game_start[0x65 + 0x24 * i] = character;
            game_start[0x66 + 0x24 * i] = 0;
            game_start[0x67 + 0x24 * i] = 4;
        }

        // Ports 3 and 4 are empty.
        game_start[0x66 + 0x24 * 2] = 3;
        game_start[0x66 + 0x24 * 3] = 3;

        game_start[0x1A5..0x1A5 + 4].copy_from_slice(b"Andy");
        // Connect code with a Shift-JIS fullwidth hash: "AA＃1".
        game_start[0x221..0x221 + 5].copy_from_slice(&[b'A', b'A', 0x81, 0x94, b'1']);
        game_start[0x2BE..0x2BE + 12].copy_from_slice(b"mode.ranked-");
        game_start[0x2F1..0x2F5].copy_from_slice(&3u32.to_be_bytes());

        stream.extend_from_slice(&game_start);

        for (player, stocks) in [(0u8, 4u8), (1u8, 3u8)] {
            let mut post_frame = vec![0u8; 0x50 + 1];
            post_frame[0] = CMD_POST_FRAME;
            post_frame[0x5] = player;
            post_frame[0x16..0x1A].copy_from_slice(&42.5f32.to_be_bytes());
            post_frame[0x21] = stocks;
            stream.extend_from_slice(&post_frame);
        }

        let mut game_end = vec![0u8; 7];
        game_end[0] = CMD_GAME_END;
        game_end[0x1] = 2;
        game_end[0x2] = 0xFF;
        game_end[0x3..0x7].copy_from_slice(&[1, 0, 0xFF_u8, 0xFF_u8]);
        stream.extend_from_slice(&game_end);

        stream
    }

    fn parse(stream: &[u8], chunk_size: usize) -> Vec<SlpEvent> {
        let mut parser = EventParser::new();
        let mut events = Vec::new();

        for chunk in stream.chunks(chunk_size) {
            parser.push(chunk, |event| events.push(event));
        }

        events
    }

    #[test]
    fn parses_a_full_game() {
        let events = parse(&test_stream(), usize::MAX);
        assert_eq!(events.len(), 4);

        let SlpEvent::GameStart(info) = &events[0] else {
            panic!("expected GameStart, got {:?}", events[0]);
        };

        assert_eq!(info.stage_id, 31);
        assert_eq!(info.match_id, "mode.ranked-");
        assert_eq!(info.game_number, 3);
        assert_eq!(info.players.len(), 2);
        assert_eq!(info.players[0].character_id, 2);
        assert_eq!(info.players[0].starting_stocks, 4);
        assert_eq!(info.players[0].display_name, "Andy");
        assert_eq!(info.players[0].connect_code, "AA#1");
        assert_eq!(info.players[1].character_id, 20);

        assert_eq!(
            events[1],
            SlpEvent::PostFrame {
                player: 0,
                stocks: 4,
                percent: 42.5
            }
        );

        assert_eq!(
            events[3],
            SlpEvent::GameEnd {
                end_method: 2,
                lras_initiator: -1,
                placements: [1, 0, -1, -1],
            }
        );
    }

    #[test]
    fn handles_arbitrary_chunk_boundaries() {
        let stream = test_stream();

        for chunk_size in [1, 7, 64, 333] {
            assert_eq!(parse(&stream, chunk_size).len(), 4, "chunk size {chunk_size}");
        }
    }

    #[test]
    fn recovers_from_a_truncated_game() {
        let stream = test_stream();
        let mut parser = EventParser::new();
        let mut events = Vec::new();

        // A game that gets cut off partway through an event...
        parser.push(&stream[..40], |event| events.push(event));
        assert_eq!(events.len(), 0);

        // ...followed by a fresh game (whole-chunk pushes, like Dolphin
        // sends them) parses cleanly.
        parser.push(&stream, |event| events.push(event));
        assert_eq!(events.len(), 4);
    }

    #[test]
    fn survives_degenerate_event_payloads() {
        let mut parser = EventParser::new();

        // A zero-length Event Payloads declaration, then random garbage, must
        // not panic (a panic on the presence thread aborts Dolphin). The
        // parser drops the bad byte and resyncs, then handles a real game.
        for stream in [&[CMD_EVENT_PAYLOADS, 0x00][..], &[CMD_EVENT_PAYLOADS, 0x00, 0xAB, 0xCD][..]] {
            parser.push(stream, |_| panic!("no events expected from garbage"));
        }

        let mut events = 0;
        parser.push(&test_stream(), |_| events += 1);
        assert_eq!(events, 4);
    }
}
