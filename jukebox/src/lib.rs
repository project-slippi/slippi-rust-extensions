use std::convert::TryInto;
use std::fmt::{Debug, Formatter};
use std::fs::File;

use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};
use hps_decode::{Hps, PcmIterator};
use rodio::{OutputStream, Sink};

mod errors;
pub use errors::JukeboxError;
use JukeboxError::*;

mod disc;
mod utils;

pub(crate) type Result<T> = std::result::Result<T, JukeboxError>;


/// By default Slippi Jukebox plays music slightly louder than vanilla melee
/// does. This reduces the overall music volume output to 80%. Not totally sure
/// if that's the correct amount, but it sounds about right.
const VOLUME_REDUCTION_MULTIPLIER: f32 = 0.8;

pub struct Jukebox {
    iso_path: String,
    _output_stream: OutputStream,
    sink: Sink,
    dolphin_system_volume: f32,
    dolphin_music_volume: f32,
    melee_volume: f32,
}

impl Jukebox {
    pub fn new(iso_path: String, dolphin_system_volume: f32, dolphin_music_volume: f32) -> Result<Self> {
        let mut iso = File::open(&iso_path)?;
        let iso_kind = disc::get_iso_kind(&mut iso)?;

        // Make sure the provided ISO is supported
        if let disc::IsoKind::Unknown = iso_kind {
            Dolphin::add_osd_message(
                Color::Red,
                OSDDuration::VeryLong,
                "\nYour ISO is not supported by Slippi Jukebox. Music will not play.",
            );
            return Err(UnsupportedIso);
        }

        // Get a handle to the default audio device
        let (output_stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        tracing::info!(target: Log::Jukebox, "Slippi Jukebox Initialized");

        Ok(Self {
            iso_path,
            _output_stream: output_stream,
            sink,
            dolphin_system_volume,
            dolphin_music_volume,
            melee_volume: 1.0,
        })
    }

    pub fn play_music(&mut self, hps_offset: u64, hps_length: usize) -> Result<()> {
        tracing::info!(
            target: Log::Jukebox,
            "Play music. Offset: 0x{hps_offset:0x?}, Length: {hps_length}"
        );

        let get_real_offset = disc::create_offset_locator_fn(&self.iso_path)?;
        let real_hps_offset = get_real_offset(hps_offset).ok_or(OffsetMissingFromCompressedIso(hps_offset))?;

        let mut iso = File::open(&self.iso_path)?;
        let hps: Hps = utils::copy_bytes_from_file(&mut iso, real_hps_offset, hps_length)?.try_into()?;
        let audio_source = HpsAudioSource(hps.into());

        self.sink.stop();
        self.sink.append(audio_source);
        self.sink.play();

        Ok(())
    }

    pub fn stop_music(&mut self) {
        tracing::info!(target: Log::Jukebox, "Stop music");

        self.sink.stop();
    }

    pub fn set_music_volume(&mut self, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Change in-game music volume: {volume}");

        self.melee_volume = (volume as f32 / 254.0).clamp(0.0, 1.0);
        self.update_sink_volume();
    }

    pub fn set_dolphin_system_volume(&mut self, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Dolphin system volume changed: {volume}");
        self.dolphin_system_volume = volume as f32 / 100.0;
        self.update_sink_volume();
    }

    pub fn set_dolphin_music_volume(&mut self, volume: u8) {
        tracing::info!(target: Log::Jukebox, "Dolphin music volume changed: {volume}");
        self.dolphin_music_volume = volume as f32 / 100.0;
        self.update_sink_volume();
    }

    fn update_sink_volume(&mut self) {
        let dolphin_volume = self.dolphin_system_volume * self.dolphin_music_volume;
        let volume = self.melee_volume * dolphin_volume * VOLUME_REDUCTION_MULTIPLIER;

        self.sink.set_volume(volume);
    }
}

impl Debug for Jukebox {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.debug_struct("Jukebox").finish()
    }
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
