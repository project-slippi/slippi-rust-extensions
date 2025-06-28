use std::convert::TryInto;
use std::fmt::Debug;
use std::fs::File;
use std::sync::mpsc::{channel, Receiver, Sender};

use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};
use hps_decode::Hps;
use rodio::{OutputStream, Sink};

use crate::Message::*;

mod errors;
pub use errors::JukeboxError;
use JukeboxError::*;

mod disc;
use disc::{get_iso_kind, IsoKind};

mod utils;
use utils::copy_bytes_from_file;

pub(crate) type Result<T> = std::result::Result<T, JukeboxError>;

/// By default Slippi Jukebox plays music slightly louder than vanilla melee
/// does. This reduces the overall music volume output to 80%. Not totally sure
/// if that's the correct amount, but it sounds about right.
const VOLUME_REDUCTION_MULTIPLIER: f32 = 0.8;

#[derive(Debug)]
pub enum Message {
    StartSong(u64, usize),
    StopMusic,
    SetVolume(VolumeControl, u8),
    JukeboxDropped,
}

#[derive(Debug)]
pub enum VolumeControl {
    Melee,
    DolphinSystem,
    DolphinMusic,
}

#[derive(Debug)]
pub struct Jukebox {
    tx: Sender<Message>,
}

impl Jukebox {
    /// Returns an instance of Slippi Jukebox. Playback can be controlled by
    /// calling the instance's public methods.
    pub fn new(iso_path: String, initial_dolphin_system_volume: u8, initial_dolphin_music_volume: u8) -> Result<Self> {
        tracing::info!(target: Log::Jukebox, "Initializing Slippi Jukebox");

        // Make sure the provided ISO is supported
        if let IsoKind::Unknown = get_iso_kind(&mut File::open(&iso_path)?)? {
            Dolphin::add_osd_message(
                Color::Red,
                OSDDuration::VeryLong,
                "\nYour ISO is not supported by Slippi Jukebox. Music will not play.",
            );
            return Err(UnsupportedIso);
        }

        // This channel allows the main thread to send messages to the
        // SlippiJukebox player thread
        let (tx, rx) = channel::<Message>();

        // Spawn the thread that will handle loading music and playing it back
        std::thread::Builder::new()
            .name("SlippiJukebox".to_string())
            .spawn(move || {
                if let Err(e) = Self::start(rx, iso_path, initial_dolphin_system_volume, initial_dolphin_music_volume) {
                    tracing::error!(
                        target: Log::Jukebox,
                        error = ?e,
                        "SlippiJukebox thread encountered an error: {e}"
                    );
                }
            })
            .map_err(ThreadSpawn)?;

        Ok(Self { tx })
    }

    /// This can be thought of as jukebox's "main" function.
    /// It runs in it's own thread on a loop, awaiting messages from the main
    /// thread. The message handlers control music playback.
    fn start(
        rx: Receiver<Message>,
        iso_path: String,
        initial_dolphin_system_volume: u8,
        initial_dolphin_music_volume: u8,
    ) -> Result<()> {
        let (_stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        let mut iso = File::open(&iso_path)?;
        let get_real_offset = disc::create_offset_locator_fn(&mut iso)?;

        let mut melee_music_volume = 1.0;
        let mut dolphin_system_volume = (initial_dolphin_system_volume as f32 / 100.0).clamp(0.0, 1.0);
        let mut dolphin_music_volume = (initial_dolphin_music_volume as f32 / 100.0).clamp(0.0, 1.0);

        sink.set_volume(melee_music_volume * dolphin_system_volume * dolphin_music_volume * VOLUME_REDUCTION_MULTIPLIER);

        loop {
            match rx.recv()? {
                StartSong(hps_offset, hps_length) => {
                    // Stop the currently playing song
                    sink.stop();

                    // Get the _real_ offset of the hps file on the iso
                    let real_hps_offset = match get_real_offset(hps_offset) {
                        Some(offset) => offset,
                        None => {
                            tracing::warn!(
                                target: Log::Jukebox,
                                "0x{hps_offset:0x?} has no corresponding offset in the ISO. Cannot play song."
                            );
                            continue;
                        },
                    };

                    // Parse the bytes as an Hps
                    let mut iso_hps = match iso.try_clone() {
                        Ok(iso) => iso,
                        Err(e) => {
                            tracing::error!(target: Log::Jukebox, error = ?e, "Failed to clone iso before reading bytes. Cannot play song.");
                            continue;
                        },
                    };
                    let hps: Hps = match copy_bytes_from_file(&mut iso_hps, real_hps_offset, hps_length)?.try_into() {
                        Ok(hps) => hps,
                        Err(e) => {
                            tracing::error!(target: Log::Jukebox, error = ?e, "Failed to parse bytes into an Hps. Cannot play song.");
                            continue;
                        },
                    };

                    // Decode the Hps into audio
                    let audio = match hps.decode() {
                        Ok(audio) => audio,
                        Err(e) => {
                            tracing::error!(target: Log::Jukebox, error = ?e, "Failed to decode hps into audio. Cannot play song.");
                            Dolphin::add_osd_message(
                                Color::Red,
                                OSDDuration::Normal,
                                "Invalid music data found in ISO. This music will not play.",
                            );
                            continue;
                        },
                    };

                    // Play the song
                    sink.append(audio);
                    sink.play();
                },
                SetVolume(control, volume) => {
                    use VolumeControl::*;

                    match control {
                        Melee => melee_music_volume = (volume as f32 / 254.0).clamp(0.0, 1.0),
                        DolphinSystem => dolphin_system_volume = (volume as f32 / 100.0).clamp(0.0, 1.0),
                        DolphinMusic => dolphin_music_volume = (volume as f32 / 100.0).clamp(0.0, 1.0),
                    };

                    sink.set_volume(
                        melee_music_volume * dolphin_system_volume * dolphin_music_volume * VOLUME_REDUCTION_MULTIPLIER,
                    );
                },
                StopMusic => sink.stop(),
                JukeboxDropped => return Ok(()),
            }
        }
    }

    /// Loads the music file in the iso at offset `hps_offset` with a length of
    /// `hps_length`, decodes it into audio, and plays it back using the default
    /// audio device
    pub fn start_song(&mut self, hps_offset: u64, hps_length: usize) {
        tracing::info!(
            target: Log::Jukebox,
            "Start song. Offset: 0x{hps_offset:0x?}, Length: {hps_length}"
        );
        let _ = self.tx.send(StartSong(hps_offset, hps_length));
    }

    /// Stops any currently playing music
    pub fn stop_music(&mut self) {
        tracing::info!(target: Log::Jukebox, "Stop music");
        let _ = self.tx.send(StopMusic);
    }

    // Update the volume for any of Jukebox's volume controls
    pub fn set_volume(&mut self, volume_control: VolumeControl, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Change {volume_control:?} volume: {volume}");
        let _ = self.tx.send(SetVolume(volume_control, volume));
    }
}

impl Drop for Jukebox {
    fn drop(&mut self) {
        tracing::info!(target: Log::Jukebox, "Dropping Slippi Jukebox");
        if let Err(e) = self.tx.send(Message::JukeboxDropped) {
            tracing::warn!(
                target: Log::Jukebox,
                "Failed to notify child thread that Jukebox is dropping: {e}"
            );
        }
    }
}
