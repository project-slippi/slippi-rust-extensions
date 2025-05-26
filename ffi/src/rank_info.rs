use std::ffi::c_float;
use std::ffi::c_uchar;
use std::ffi::c_uint;
use std::ffi::c_int;
use slippi_exi_device::SlippiEXIDevice;
use dolphin_integrations::Log;
use slippi_rank_info::SlippiRank;

use slippi_rank_info::RankManager;

use crate::{with_returning};

#[repr(C)]
pub struct RustRankInfo {
    pub rank: c_uchar,
    pub rating_ordinal: c_float,
    pub global_placing: c_uchar,
    pub regional_placing: c_uchar,
    pub rating_update_count: c_uint,
    pub rating_change: c_float,
    pub rank_change: c_int,
}

/// Fetches the rank information of the user currently logged in.
#[no_mangle]
pub extern "C" fn slprs_get_rank_info(exi_device_instance_ptr: usize) -> *mut RustRankInfo {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let rank_info = device.user_manager.get(|user| {

            // Get cached rank data if it exists
            let prev_rank= match &device.rank_manager.last_rank {
                Some(last_rank) => last_rank.rank as i8,
                None => 0
            };
            let prev_rating_ordinal= match &device.rank_manager.last_rank {
                Some(last_rank) => last_rank.rating_ordinal,
                None => 0.0 
            };

            // TODO :: return cached rank if request fails
            let connect_code_str = user.connect_code.as_str();
            match RankManager::fetch_user_rank(&mut device.rank_manager, connect_code_str) {
                Ok(value) => {
                    let has_cached_rating = prev_rating_ordinal != 0.0;
                    let has_cached_rank = prev_rank != 0;

                    let rating_change: f32 =
                        if has_cached_rating { 
                            value.rating_ordinal - prev_rating_ordinal
                        } else { 0.0 };

                    let curr_rating_ordinal = 
                        if !has_cached_rating { 
                            value.rating_ordinal 
                        } else { 
                            prev_rating_ordinal 
                        };

                    let curr_rank = 
                        RankManager::get_rank(
                            value.rating_ordinal, 
                            value.global_placing, 
                            value.regional_placing, 
                            value.rating_update_count
                        ) as i8;

                    let rank_change: i8 = 
                        if has_cached_rank { 
                            curr_rank - prev_rank 
                        } else { 0 };

                    Box::new(RustRankInfo {
                        rank: (curr_rank - rank_change) as c_uchar,
                        rating_ordinal: curr_rating_ordinal as c_float,
                        global_placing: value.global_placing,
                        regional_placing: value.regional_placing,
                        rating_update_count: value.rating_update_count,
                        rating_change: rating_change,
                        rank_change: rank_change as c_int,
                    })
                }
                Err(err) => {
                    tracing::error!(target: Log::SlippiOnline, "Failed to fetch rank: {:?}", err);
                    
                    Box::new(RustRankInfo {
                        rank: 0,
                        rating_ordinal: 0.0,
                        global_placing: 0,
                        regional_placing: 0,
                        rating_update_count: 0,
                        rating_change: 0.0,
                        rank_change: 0,
                    })
                }
            }
        });
        Box::into_raw(rank_info)
    })
}