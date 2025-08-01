/// Represents a rank in the Slippi playerbase.
#[repr(i8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlippiRank {
    Unranked,
    Bronze1,
    Bronze2,
    Bronze3,
    Silver1,
    Silver2,
    Silver3,
    Gold1,
    Gold2,
    Gold3,
    Platinum1,
    Platinum2,
    Platinum3,
    Diamond1,
    Diamond2,
    Diamond3,
    Master1,
    Master2,
    Master3,
    Grandmaster,
}

/// Determines the current `SlippiRank` given the provided values.
pub fn decide(rating_ordinal: f32, global_placing: u8, regional_placing: u8, rating_update_count: u32) -> SlippiRank {
    if rating_update_count < 5 {
        return SlippiRank::Unranked;
    }

    if rating_ordinal > 0.0 && rating_ordinal <= 765.42 {
        return SlippiRank::Bronze1;
    }

    if rating_ordinal > 765.43 && rating_ordinal <= 913.71 {
        return SlippiRank::Bronze2;
    }

    if rating_ordinal > 913.72 && rating_ordinal <= 1054.86 {
        return SlippiRank::Bronze3;
    }

    if rating_ordinal > 1054.87 && rating_ordinal <= 1188.87 {
        return SlippiRank::Silver1;
    }

    if rating_ordinal > 1188.88 && rating_ordinal <= 1315.74 {
        return SlippiRank::Silver2;
    }

    if rating_ordinal > 1315.75 && rating_ordinal <= 1435.47 {
        return SlippiRank::Silver3;
    }

    if rating_ordinal > 1435.48 && rating_ordinal <= 1548.06 {
        return SlippiRank::Gold1;
    }

    if rating_ordinal > 1548.07 && rating_ordinal <= 1653.51 {
        return SlippiRank::Gold2;
    }
    if rating_ordinal > 1653.52 && rating_ordinal <= 1751.82 {
        return SlippiRank::Gold3;
    }

    if rating_ordinal > 1751.83 && rating_ordinal <= 1842.99 {
        return SlippiRank::Platinum1;
    }

    if rating_ordinal > 1843.0 && rating_ordinal <= 1927.02 {
        return SlippiRank::Platinum2;
    }

    if rating_ordinal > 1927.03 && rating_ordinal <= 2003.91 {
        return SlippiRank::Platinum3;
    }

    if rating_ordinal > 2003.92 && rating_ordinal <= 2073.66 {
        return SlippiRank::Diamond1;
    }

    if rating_ordinal > 2073.67 && rating_ordinal <= 2136.27 {
        return SlippiRank::Diamond2;
    }

    if rating_ordinal > 2136.28 && rating_ordinal <= 2191.74 {
        return SlippiRank::Diamond3;
    }

    if rating_ordinal >= 2191.75 && global_placing > 0 && regional_placing > 0 {
        return SlippiRank::Grandmaster;
    }

    if rating_ordinal > 2191.75 && rating_ordinal <= 2274.99 {
        return SlippiRank::Master1;
    }

    if rating_ordinal > 2275.0 && rating_ordinal <= 2350.0 {
        return SlippiRank::Master2;
    }

    if rating_ordinal > 2350.0 {
        return SlippiRank::Master3;
    }

    SlippiRank::Unranked
}
