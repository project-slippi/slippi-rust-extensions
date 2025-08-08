use std::ffi::{c_char, c_float, c_int, c_uint};

use slippi_exi_device::SlippiEXIDevice;
use slippi_user::RankInfo;

use crate::c_str_to_string;
use crate::with_returning;

/// Rank info that we vend back to the Dolphin side of things.
#[repr(C)]
pub struct RustRankInfo {
    pub fetch_status: c_int,
    pub rank: c_char,
    pub rating_ordinal: c_float,
    pub rating_update_count: c_uint,
    pub rating_change: c_float,
    pub rank_change: c_int,
}

/// Fetches the result of a recently played match via its ID.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_fetch_match_result(exi_device_instance_ptr: usize, match_id: *const c_char) {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let fn_name = "slprs_fetch_match_result";
        let match_id = c_str_to_string(match_id, fn_name, "match_id");
        device.user_manager.fetch_match_result(match_id);
    })
}

/// Gets the most recently fetched rank information of the user currently logged in.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_get_rank_info(exi_device_instance_ptr: usize) -> RustRankInfo {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let (rank_opt, fetch_status) = device.user_manager.current_rank_and_status();
        let rank = rank_opt.unwrap_or({
            let mut default = RankInfo::default();
            default.rank = -1;
            default
        });

        RustRankInfo {
            fetch_status: fetch_status as c_int,
            rank: rank.rank as c_char,
            rating_ordinal: rank.rating_ordinal as c_float,
            rating_update_count: rank.rating_update_count as c_uint,
            rating_change: rank.rating_change as c_float,
            rank_change: rank.rank_change as c_int,
        }
    })
}
