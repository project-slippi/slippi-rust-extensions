# Slippi Discord Rich Presence

Mirrors live Slippi online game state to a locally running Discord client: the
stage, characters, live stock counts and set score during a match, plus the
menu, queue and matchmaking flow between matches.

## How it works

Two data sources feed the presence, both on a single background thread
(`SlippiDiscordRpc`) owned by `DiscordHandler`. Neither reads emulated memory.

1. **The replay event stream.** Dolphin already tees the replay payloads the
   EXI device receives into this crate. An incremental parser (`parser.rs`)
   pulls out `Game Start` (stage, characters, names, match ID), `Post Frame`
   (live stock counts) and `Game End` (placements, which drive the set score).

2. **Slippi's matchmaking state** (`menu.rs`). Between games the C++ side pushes
   the matchmaking state over `slprs_exi_device_update_matchmaking_state` each
   online frame, straight from Slippi's own `SlippiMatchmaking`. The thread
   edge-detects and only re-renders when it changes.

Discord I/O is local IPC (Unix sockets on macOS/Linux, named pipes on Windows)
via the `discord-rich-presence` crate; there is no networking here. If Discord
isn't running, the thread retries and the rest of Slippi is unaffected. The
rank shown while queueing comes from the `slippi-user` crate's `UserManager`,
and is hidden when the player has hidden their rank in-game
(`SLIPPI_ENABLE_RANK_LOCAL`).

## Testing without Dolphin

Both halves can be exercised against a real Discord client with no Dolphin
build:

```sh
# Stream a real .slp replay through the parser at 8x speed.
cargo run -p slippi-discord-rpc --example simulate -- path/to/replay.slp 8

# Walk menus, queue and opponent-found by pushing matchmaking snapshots.
cargo run -p slippi-discord-rpc --example simulate_queue

# Print what the parser sees in a replay.
cargo run -p slippi-discord-rpc --example dump -- path/to/replay.slp

cargo test -p slippi-discord-rpc
```

## Dolphin integration

Like the Jukebox, the Rust lives here and the small game-side glue lives in the
Dolphin repo, joined by the cbindgen FFI header. The C++ side:

- Tees `slprs_exi_device_reporter_push_replay_data` into this crate (already
  called today; nothing new for in-game presence).
- Calls `slprs_exi_device_configure_discord_rpc(ptr, is_enabled, show_rank)`
  once, gating `is_enabled` on a `SLIPPI_ENABLE_DISCORD_RPC` setting (default
  on, with a Slippi-pane checkbox like `SLIPPI_ENABLE_JUKEBOX`) and passing
  `show_rank` from `SLIPPI_ENABLE_RANK_LOCAL`.
- Calls `slprs_exi_device_update_matchmaking_state(ptr, process_state,
  online_mode, opponent_name, opponent_rank)` each online frame from
  `prepareOnlineMatchState()`.
- Registers a `SLIPPI_RUST_DISCORD_RPC` log container in `LogManager.cpp`, like
  `SLIPPI_RUST_JUKEBOX`.

These edits ship as a small PR against
[project-slippi/dolphin](https://github.com/project-slippi/dolphin).

The Discord application ID and its image assets (`char{id}`, `stage{id}`, rank
badges) are documented at the top of `presence.rs`. Two buttons, "Get Slippi"
and a link to the player's `slippi.gg` profile, ride along on every update;
Discord shows them only to other people viewing the profile.

## Caveats

- Only the online flow is covered. Offline modes (training, VS) and non-match
  scenes have no matchmaking state and don't feed the replay stream, so they
  show the generic "in menus".
- The `ProcessState` / `OnlinePlayMode` integer values mirror Slippi's
  `SlippiMatchmaking` enums, so the two stay in lockstep.
