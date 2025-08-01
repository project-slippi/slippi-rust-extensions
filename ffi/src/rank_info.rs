use std::ffi::{c_char, c_float, c_int, c_uchar, c_uint};

use slippi_exi_device::SlippiEXIDevice;
use slippi_rank_info::RankInfo;

use crate::with_returning;

/// Rank info that we vend back to the Dolphin side of things.
#[repr(C)]
pub struct RustRankInfo {
    pub rank: c_char,
    pub rating_ordinal: c_float,
    pub global_placing: c_uchar,
    pub regional_placing: c_uchar,
    pub rating_update_count: c_uint,
    pub rating_change: c_float,
    pub rank_change: c_int,
}

/// Fetches the rank information of the user currently logged in.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_fetch_rank_info(exi_device_instance_ptr: usize) {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        device.rank_manager.fetch_rank();
    })
}

/// Gets the most recently fetched rank information of the user currently logged in.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_get_rank_info(exi_device_instance_ptr: usize) -> RustRankInfo {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let rank = device.rank_manager.get_rank().unwrap_or({
            let mut default = RankInfo::default();
            default.rank = -1;
            default
        });

        RustRankInfo {
            rank: rank.rank as c_char,
            rating_ordinal: rank.rating_ordinal as c_float,
            global_placing: rank.global_placing as c_uchar,
            regional_placing: rank.regional_placing as c_uchar,
            rating_update_count: rank.rating_update_count as c_uint,
            rating_change: rank.rating_change as c_float,
            rank_change: rank.rank_change as c_int,
        }
    })
}
