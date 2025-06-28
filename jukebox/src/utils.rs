use std::collections::HashMap;
use std::ffi::CStr;
use std::fs::{read_dir, File};
use std::io::{BufReader, Read, Seek};
use std::path::PathBuf;

use dolphin_integrations::Log;
use rodio::Decoder;

use crate::disc::create_offset_locator_fn;
use crate::{
    JukeboxError::{self, *},
    Result,
};

/// Get a copy of the `size` bytes in `file` at `offset`
pub(crate) fn copy_bytes_from_file(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>> {
    file.seek(std::io::SeekFrom::Start(offset)).map_err(IsoSeek)?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(IsoRead)?;
    Ok(bytes)
}

/// Converts a `.hps` to its equivalent stage name
pub(crate) fn hps_to_stage(hps: &str) -> Option<String> {
    let stage = match hps {
        // Legal stages
        "hyaku2.hps" | "sp_end.hps" => "final_destination",
        "hyaku.hps" | "sp_zako.hps" => "battlefield",
        "ystory.hps" => "yoshis_story",
        "izumi.hps" => "fountain_of_dreams",
        "old_kb.hps" => "dreamland",
        "pokesta.hps" | "pstadium.hps" => "pokemon_stadium",
        // Misc
        "menu3.hps" | "menu02.hps" | "menu01.hps" => "menu",
        "target.hps" => "target_test",
        // Other
        "yorster.hps" | "smari3.hps" => "yoshis_island",
        "old_ys.hps" => "yoshis_island_2",
        "old_dk.hps" => "kongo_jungle",
        "kraid.hps" => "brinstar_depths",
        "fourside.hps" => "fourside",
        "bigblue.hps" | "mrider.hps" => "big_blue",
        "pura.hps" => "poke_floats",
        "inis1_01.hps" | "docmari.hps" => "kingdom",
        "inis2_01.hps" => "kingdom_2",
        "zebes.hps" => "brinstar",
        "onetto2.hps" | "onetto.hps" => "onett",
        "mutecity.hps" => "mute_city",
        "rcruise.hps" => "rainbow_cruise",
        "kongo.hps" => "jungle_japes",
        "shrine.hps" => "temple",
        "greens.hps" => "green_greens",
        "venom.hps" => "venom",
        "baloon.hps" | "icemt.hps" => "icicle_mountain",
        "castle.hps" => "princess_peachs_castle",
        "garden.hps" => "kongo_jungle",
        "saria.hps" | "greatbay.hps" => "great_bay",
        "corneria.hps" => "corneria",
        "flatzone.hps" => "flatzone",
        _ => "",
    };

    if !stage.is_empty() {
        Some(stage.to_owned())
    } else {
        None
    }
}

pub struct TrackList {
    track_map: HashMap<u64, PathBuf>,
}

impl TrackList {
    pub fn new(mut iso: &mut File, jukebox_path: PathBuf) -> Option<TrackList> {
        let mut track_map = HashMap::new();

        const RAW_FST_LOCATION_OFFSET: u64 = 0x424;
        const RAW_FST_SIZE_OFFSET: u64 = 0x428;
        const FST_ENTRY_SIZE: usize = 0xC;

        let get_true_offset = create_offset_locator_fn(&mut iso).ok()?;
        let fst_location_offset = get_true_offset(RAW_FST_LOCATION_OFFSET)?;
        let fst_size_offset = get_true_offset(RAW_FST_SIZE_OFFSET)?;

        let fst_location = u32::from_be_bytes(
            copy_bytes_from_file(&mut iso, fst_location_offset as u64, 0x4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let fst_location = get_true_offset(fst_location as u64).unwrap();

        if fst_location > 0 {
            let fst_size = u32::from_be_bytes(
                copy_bytes_from_file(&mut iso, fst_size_offset as u64, 0x4)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            );

            let fst = copy_bytes_from_file(&mut iso, fst_location as u64, fst_size as usize).unwrap();

            // FST String Table
            let str_table_offset = read_u32(&fst, 0x8) as usize * FST_ENTRY_SIZE;

            // Collect the .hps file metadata in the FST into a hash map
            for entry in fst[..str_table_offset].chunks(FST_ENTRY_SIZE).into_iter() {
                let is_file = entry[0] == 0;
                let name_offset = str_table_offset + read_u24(entry, 0x1) as usize;
                let offset = read_u32(entry, 0x4) as u64;

                let name = CStr::from_bytes_until_nul(&fst[name_offset..]).unwrap().to_str().unwrap();

                if is_file && name.ends_with(".hps") {
                    if let Some(stage) = hps_to_stage(name) {
                        track_map.insert(offset, jukebox_path.join(stage));
                    }
                }
            }
        }

        Some(TrackList { track_map })
    }

    /// Attempts to find a custom song for the specified offset's `.hps` owning stage
    pub fn find_custom_song(&self, offset: u64) -> Option<Decoder<BufReader<File>>> {
        // Find track matching offset
        let stage_dir = &self.track_map.get(&offset)?;

        // Get all files in folder
        let entries = read_dir(&stage_dir).ok()?;
        let files: Vec<_> = entries
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.is_file() {
                    let extension = path.extension()?.to_str()?.to_lowercase();
                    match extension.as_str() {
                        "mp3" | "wav" | "ogg" | "flac" => return Some(path),
                        _ => return None,
                    }
                }

                None
            })
            .collect();

        // Choose a random file from the stage folder if available
        if !files.is_empty() {
            let random_path = fastrand::choice(files.iter())?;
            match File::open(random_path) {
                Ok(custom_song_file) => {
                    if let Ok(custom_song) = rodio::Decoder::new(BufReader::new(custom_song_file)) {
                        return Some(custom_song);
                    }
                },
                Err(e) => {
                    tracing::error!(target: Log::Jukebox, error = ?e, "Failed to open custom song. Cannot play song.");
                },
            }
        }

        None
    }
}

/// Get an unsigned 24 bit integer from a byte slice
fn read_u24(bytes: &[u8], offset: usize) -> u32 {
    let size = 3;
    let end = offset + size;
    let mut padded_bytes = [0; 4];
    let slice = &bytes.get(offset..end).unwrap();
    padded_bytes[1..4].copy_from_slice(slice);

    u32::from_be_bytes(padded_bytes)
}

/// Get an unsigned 32 bit integer from a byte slice
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let size = (u32::BITS / 8) as usize;
    let end: usize = offset + size;
    u32::from_be_bytes(
        bytes
            .get(offset..end)
            .unwrap()
            .try_into()
            .unwrap_or_else(|_| unreachable!("u32::BITS / 8 is always 4")),
    )
}
