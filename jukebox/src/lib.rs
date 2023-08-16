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

/// Represents a foreign method from the Dolphin side for grabbing the current volume.
/// Dolphin represents this as a number from 0 - 100; 0 being mute.
pub type ForeignGetVolumeFn = unsafe extern "C" fn() -> std::ffi::c_int;

/// By default Slippi Jukebox plays music slightly louder than vanilla melee
/// does. This reduces the overall music volume output to 80%. Not totally sure
/// if that's the correct amount, but it sounds about right.
const VOLUME_REDUCTION_MULTIPLIER: f32 = 0.8;

pub struct Jukebox {
    iso_path: String,
    _output_stream: OutputStream,
    sink: Sink,
    get_dolphin_volume_fn: ForeignGetVolumeFn,
}

impl Jukebox {
    pub fn new(iso_path: String, get_dolphin_volume_fn: ForeignGetVolumeFn) -> Result<Self> {
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
            get_dolphin_volume_fn,
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

        let melee_volume = (volume as f32 / 254.0).clamp(0.0, 1.0);
        let dolphin_volume = unsafe { (self.get_dolphin_volume_fn)() as f32 / 100.0 };
        let volume = melee_volume * dolphin_volume * VOLUME_REDUCTION_MULTIPLIER;

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
