//! Melee ID lookups (characters and stages) and the Discord asset names they
//! map to. IDs are the external IDs from the Slippi `Game Start` event, not the
//! in-game internal IDs.
//! See https://github.com/project-slippi/slippi-wiki/blob/master/SPEC.md

/// Returns the display name for an external character ID, if known.
pub(crate) fn character_name(id: u8) -> Option<&'static str> {
    Some(match id {
        0x00 => "Captain Falcon",
        0x01 => "Donkey Kong",
        0x02 => "Fox",
        0x03 => "Mr. Game & Watch",
        0x04 => "Kirby",
        0x05 => "Bowser",
        0x06 => "Link",
        0x07 => "Luigi",
        0x08 => "Mario",
        0x09 => "Marth",
        0x0A => "Mewtwo",
        0x0B => "Ness",
        0x0C => "Peach",
        0x0D => "Pikachu",
        0x0E => "Ice Climbers",
        0x0F => "Jigglypuff",
        0x10 => "Samus",
        0x11 => "Yoshi",
        0x12 => "Zelda",
        0x13 => "Sheik",
        0x14 => "Falco",
        0x15 => "Young Link",
        0x16 => "Dr. Mario",
        0x17 => "Roy",
        0x18 => "Pichu",
        0x19 => "Ganondorf",
        _ => return None,
    })
}

/// Returns the Discord asset key for an external character ID.
pub(crate) fn character_asset(id: u8) -> String {
    match character_name(id) {
        Some(_) => format!("char{id}"),
        None => "questionmark".to_string(),
    }
}

/// Returns the display name for an internal stage ID, the value Melee keeps for
/// the currently loaded stage (as opposed to the external `Game Start` ID).
pub(crate) fn stage_name_internal(internal_id: u16) -> Option<&'static str> {
    Some(match internal_id {
        2 => "Princess Peach's Castle",
        3 => "Rainbow Cruise",
        4 => "Kongo Jungle",
        5 => "Jungle Japes",
        6 => "Great Bay",
        7 => "Temple",
        8 => "Brinstar",
        9 => "Brinstar Depths",
        10 => "Yoshi's Story",
        11 => "Yoshi's Island",
        12 => "Fountain of Dreams",
        13 => "Green Greens",
        14 => "Corneria",
        15 => "Venom",
        16 => "Pokémon Stadium",
        17 => "Poké Floats",
        18 => "Mute City",
        19 => "Big Blue",
        20 => "Onett",
        21 => "Fourside",
        22 => "Icicle Mountain",
        24 => "Mushroom Kingdom",
        25 => "Mushroom Kingdom II",
        27 => "Flat Zone",
        28 => "Dream Land",
        29 => "Yoshi's Island (N64)",
        30 => "Kongo Jungle (N64)",
        36 => "Battlefield",
        37 => "Final Destination",
        _ => return None,
    })
}

/// Maps an external (`Game Start` replay) stage ID to its internal ID.
fn external_to_internal(external_id: u16) -> Option<u16> {
    Some(match external_id {
        2 => 12,
        3 => 16,
        4 => 2,
        5 => 4,
        6 => 8,
        7 => 14,
        8 => 10,
        9 => 20,
        10 => 18,
        11 => 3,
        12 => 5,
        13 => 6,
        14 => 7,
        15 => 9,
        16 => 11,
        17 => 13,
        18 => 21,
        19 => 24,
        20 => 25,
        22 => 15,
        23 => 17,
        24 => 19,
        25 => 22,
        27 => 27,
        28 => 28,
        29 => 29,
        30 => 30,
        31 => 36,
        32 => 37,
        _ => return None,
    })
}

/// Returns the display name for an external stage ID, if known.
pub(crate) fn stage_name(external_id: u16) -> Option<&'static str> {
    stage_name_internal(external_to_internal(external_id)?)
}

/// Returns the Discord asset key for an external stage ID.
pub(crate) fn stage_asset(external_id: u16) -> String {
    match external_to_internal(external_id) {
        Some(internal_id) => format!("stage{internal_id}"),
        None => "questionmark".to_string(),
    }
}

/// Returns the Discord asset key for an internal stage ID. The stage assets are
/// keyed by internal ID, so this matches what [`stage_asset`] produces.
pub(crate) fn stage_asset_internal(internal_id: u16) -> String {
    match stage_name_internal(internal_id) {
        Some(_) => format!("stage{internal_id}"),
        None => "questionmark".to_string(),
    }
}
