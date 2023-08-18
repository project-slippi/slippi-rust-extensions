use std::convert::TryInto;
use std::fmt::Debug;
use std::fs::File;
use std::sync::mpsc::{channel, Receiver, Sender};

use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};
use hps_decode::{Hps, PcmIterator};
use rodio::{OutputStream, Sink};

use crate::Message::*;

mod errors;
pub use errors::JukeboxError;
use JukeboxError::*;

mod disc;
use disc::{get_iso_kind, get_real_offset, IsoKind};

mod utils;

pub(crate) type Result<T> = std::result::Result<T, JukeboxError>;

/// By default Slippi Jukebox plays music slightly louder than vanilla melee
/// does. This reduces the overall music volume output to 80%. Not totally sure
/// if that's the correct amount, but it sounds about right.
const VOLUME_REDUCTION_MULTIPLIER: f32 = 0.8;

#[derive(Debug)]
pub enum Message {
    StartSong(u64, usize),
    StopMusic,
    SetMeleeMusicVolume(u8),
    SetDolphinSystemVolume(u8),
    SetDolphinMusicVolume(u8),
    JukeboxDropped,
}

#[derive(Debug)]
pub struct Jukebox {
    tx: Sender<Message>,
}

impl Jukebox {
    /// Returns an instance of Slippi Jukebox. Playback can be controlled by
    /// calling the instance's public methods.
    pub fn new(iso_path: String, dolphin_system_volume: f32, dolphin_music_volume: f32) -> Result<Self> {
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
                if let Err(e) = Self::start(rx, iso_path, dolphin_system_volume, dolphin_music_volume) {
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
    fn start(rx: Receiver<Message>, iso_path: String, dolphin_system_volume: f32, dolphin_music_volume: f32) -> Result<()> {
        let (_stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        let mut iso = File::open(&iso_path)?;

        let mut volume = Volume {
            melee_music: 1.0,
            dolphin_system: dolphin_system_volume,
            dolphin_music: dolphin_music_volume,
        };

        let set_sink_volume =
            |v: &Volume| sink.set_volume(v.melee_music * v.dolphin_music * v.dolphin_system * VOLUME_REDUCTION_MULTIPLIER);

        loop {
            match rx.recv()? {
                StartSong(hps_offset, hps_length) => {
                    let real_hps_offset =
                        get_real_offset(&mut iso, hps_offset)?.ok_or(OffsetMissingFromCompressedIso(hps_offset))?;

                    let hps: Hps = utils::copy_bytes_from_file(&mut iso, real_hps_offset, hps_length)?.try_into()?;
                    let audio_source = HpsAudioSource(hps.into());

                    sink.stop();
                    sink.append(audio_source);
                    sink.play();
                },
                SetMeleeMusicVolume(value) => {
                    volume.melee_music = (value as f32 / 254.0).clamp(0.0, 1.0);
                    set_sink_volume(&volume);
                },
                SetDolphinSystemVolume(value) => {
                    volume.dolphin_system = (value as f32 / 100.0).clamp(0.0, 1.0);
                    set_sink_volume(&volume);
                },
                SetDolphinMusicVolume(value) => {
                    volume.dolphin_music = (value as f32 / 100.0).clamp(0.0, 1.0);
                    set_sink_volume(&volume);
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
        self.tx.send(StartSong(hps_offset, hps_length)).ok();
    }

    /// Stops any currently playing music
    pub fn stop_music(&mut self) {
        tracing::info!(target: Log::Jukebox, "Stop music");
        self.tx.send(StopMusic).ok();
    }

    /// Indicate to the jukebox instance that melee's in-game volume has changed
    pub fn set_melee_music_volume(&mut self, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Change in-game music volume: {volume}");
        self.tx.send(SetMeleeMusicVolume(volume)).ok();
    }

    /// Indicate to the jukebox instance that Dolphin's audio config volume has
    /// changed
    pub fn set_dolphin_system_volume(&mut self, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Change dolphin audio config volume: {volume}");
        self.tx.send(SetDolphinSystemVolume(volume)).ok();
    }

    /// Indicate to the jukebox instance that Dolphin's "Jukebox volume" slider
    /// value has changed
    pub fn set_dolphin_music_volume(&mut self, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Change jukebox music volume: {volume}");
        self.tx.send(SetDolphinMusicVolume(volume)).ok();
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

struct Volume {
    melee_music: f32,
    dolphin_music: f32,
    dolphin_system: f32,
}

// This wrapper allows us to implement `rodio::Source`
struct HpsAudioSource(PcmIterator);

impl Iterator for HpsAudioSource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl rodio::Source for HpsAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.0.channel_count as u16
    }
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}
