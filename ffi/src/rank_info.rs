use slippi_exi_device::SlippiEXIDevice;
use std::ffi::c_char;
use std::ffi::c_float;
use std::ffi::c_int;
use std::ffi::c_uchar;
use std::ffi::c_uint;

use crate::with_returning;

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
        let rank_info = match device.rank_manager.get_rank() {
            Some(curr_rank) => RustRankInfo {
                rank: curr_rank.rank as c_char,
                rating_ordinal: curr_rank.rating_ordinal as c_float,
                global_placing: curr_rank.global_placing as c_uchar,
                regional_placing: curr_rank.regional_placing as c_uchar,
                rating_update_count: curr_rank.rating_update_count as c_uint,
                rating_change: curr_rank.rating_change as c_float,
                rank_change: curr_rank.rank_change as c_int,
            },
            None => RustRankInfo {
                rank: -1 as c_char, // Send invalid rank if data is empty
                rating_ordinal: 0.0 as c_float,
                global_placing: 0 as c_uchar,
                regional_placing: 0 as c_uchar,
                rating_update_count: 0 as c_uint,
                rating_change: 0.0 as c_float,
                rank_change: 0 as c_int,
            },
        };
        rank_info
    })
}
