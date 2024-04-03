use std::fs::File;
use std::io::{Read, Seek};

use crate::{JukeboxError::*, Result};

/// Get a copy of the `size` bytes in `file` at `offset`
pub(crate) fn copy_bytes_from_file(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>> {
    file.seek(std::io::SeekFrom::Start(offset)).map_err(IsoSeek)?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(IsoRead)?;
    Ok(bytes)
}

/// Converts an HPS offset to its equivalent stage name
/// Essentially, gets the stage belonging to a song
pub(crate) fn hps_to_stage(offset: u64) -> Option<String> {
    let stage = match offset {
        // Legal stages
        71324300 | 183962648 => "final_destination",
        67606252 | 192481656 => "battlefield",
        260526192 => "yoshis_story",
        86968172 => "fountain_of_dreams",
        135559532 => "dreamland",
        156902380 | 160483852 => "pokemon_stadium",
        // Misc
        116194572 | 114173548 | 112087660 => "menu",
        227529496 => "target_test",
        // Other
        258226448 | 180321356 => "yoshis_island",
        138041836 => "yoshis_island_2",
        129809196 => "kongo_jungle",
        102329228 => "brinstar_depths",
        40067628 => "fourside",
        12767116 | 119550124 => "big_blue",
        163012812 => "poke_floats",
        79192940 | 24668300 => "kingdom",
        83914380 => "kingdom_2",
        264873488 => "brinstar",
        144477996 | 140261388 => "onett",
        124345132 => "mute_city",
        168831468 => "rainbow_cruise",
        94385004 => "jungle_japes",
        173785356 => "temple",
        56785644 => "green_greens",
        250920912 => "venom",
        10628844 | 75159404 => "icicle_mountain",
        17422220 => "princess_peachs_castle",
        45632108 => "kongo_jungle",
        172460780 | 54558348 => "great_bay",
        21447116 => "corneria",
        36096908 => "flatzone",
        _ => "",
    };

    if !stage.is_empty() {
        Some(stage.to_owned())
    } else {
        None
    }
}
