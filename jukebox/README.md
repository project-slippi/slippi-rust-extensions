# Slippi Jukebox

Slippi Jukebox serves as an integrated solution for playing Melee's OST in a way that's effectively independent from emulation.

## How music is controlled

Three injections have been added game-side (not in this repo) to enable Jukebox to function. These injections send messages to the Slippi EXI Device:

- The first is in the `fileLoad_HPS` function. This runs once whenever the game is about to play a new song.
- The second is in `Music_StopMusic`, which runs whenever the game wants to stop music playback.
- The third is `DSP_Process`. This injection runs right after the game finishes calculating the "final music volume" variable which includes sound setting, pause multiplier, starman multiplier, etc. The function runs once per frame, so the previous volume is stored - this allows a message to be sent only when the value has changed.

The Slippi EXI Device will forward these messages to the Rust EXI device which may or may not be holding an instance of Jukebox (depending on if the player has music enabled or not.)

## How music is played back

When a `Jukebox` instance is created it will spawn a child thread which loops waiting to receive messages from the main thread. When a message to play a song is received, it reads from disk (completely independent of Dolphin) to load music data from the iso, [decodes it into audio](#decoding-melees-music) and plays it back with with the default audio device.

When the `Jukebox` instance is dropped, the child thread terminates and the music stops.

## Decoding Melee's Music

The logic for decoding Melee's music has been split out into a public library. See the [`hps_decode`](https://crates.io/crates/hps_decode) crate for more. For general information about the `.hps` file format, [see here.](https://github.com/DarylPinto/hps_decode/blob/main/HPS-LAYOUT.md)
