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

impl SlippiRank {
    /// Decides the rank based on the values provided.
    pub fn decide(rating_ordinal: f32, global_placing: u16, regional_placing: u16, rating_update_count: u32) -> Self {
        if rating_update_count < 5 {
            return SlippiRank::Unranked;
        }

        // Each threshold is the minimum rating for that rank. Checking from the top down means every
        // rating lands in exactly one rank, including negative ones (Bronze1). This must match the
        // launcher and website.
        if rating_ordinal >= 2191.75 && (global_placing > 0 || regional_placing > 0) {
            return SlippiRank::Grandmaster;
        }

        if rating_ordinal >= 2350.0 {
            return SlippiRank::Master3;
        }

        if rating_ordinal >= 2275.0 {
            return SlippiRank::Master2;
        }

        if rating_ordinal >= 2191.75 {
            return SlippiRank::Master1;
        }

        if rating_ordinal >= 2136.28 {
            return SlippiRank::Diamond3;
        }

        if rating_ordinal >= 2073.67 {
            return SlippiRank::Diamond2;
        }

        if rating_ordinal >= 2003.92 {
            return SlippiRank::Diamond1;
        }

        if rating_ordinal >= 1927.03 {
            return SlippiRank::Platinum3;
        }

        if rating_ordinal >= 1843.0 {
            return SlippiRank::Platinum2;
        }

        if rating_ordinal >= 1751.83 {
            return SlippiRank::Platinum1;
        }

        if rating_ordinal >= 1653.52 {
            return SlippiRank::Gold3;
        }

        if rating_ordinal >= 1548.07 {
            return SlippiRank::Gold2;
        }

        if rating_ordinal >= 1435.48 {
            return SlippiRank::Gold1;
        }

        if rating_ordinal >= 1315.75 {
            return SlippiRank::Silver3;
        }

        if rating_ordinal >= 1188.88 {
            return SlippiRank::Silver2;
        }

        if rating_ordinal >= 1054.87 {
            return SlippiRank::Silver1;
        }

        if rating_ordinal >= 913.72 {
            return SlippiRank::Bronze3;
        }

        if rating_ordinal >= 765.43 {
            return SlippiRank::Bronze2;
        }

        SlippiRank::Bronze1
    }
}
