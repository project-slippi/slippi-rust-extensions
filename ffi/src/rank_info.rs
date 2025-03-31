use std::ffi::c_char;
use std::ffi::c_float;
use std::ffi::c_uchar;
use std::ffi::c_uint;
use std::sync::Arc;
use std::sync::Mutex;
use std::ffi::{c_int, CString};
use slippi_exi_device::SlippiEXIDevice;
use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};

use slippi_rank_info::RankManager;

use crate::{c_str_to_string, with, with_returning};

#[repr(C)]
pub struct RustRankInfo {
    pub rank: c_uchar,
    pub rating_ordinal: c_float,
    pub global_placing: c_uchar,
    pub regional_placing: c_uchar,
    pub rating_update_count: c_uint,
    pub rating_change: c_float,
    pub rank_change: c_char,
}

/// Creates a new Player Report and leaks it, returning the pointer.
///
/// This should be passed on to a GameReport for processing.
#[no_mangle]
pub extern "C" fn slprs_get_rank_info(exi_device_instance_ptr: usize) -> *mut RustRankInfo {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let rank_info = device.user_manager.get(|user| {
            let connect_code_str = user.connect_code.as_str();

            let mut prev_rating_ordinal= match &device.rank_info.last_rank {
                Some(last_rank) => {
                    tracing::info!(target: Log::SlippiOnline, "last rating: {}", last_rank.rating_ordinal);
                    last_rank.rating_ordinal
                },
                None => 0.0 
            };

            let res = RankManager::fetch_user_rank(&mut device.rank_info, connect_code_str);

            match res {
                Ok(value) => {
                    let curr_rank = RankManager::get_rank(
                            value.rating_ordinal, 
                            value.global_placing, 
                            value.regional_placing, 
                            value.rating_update_count
                        ) as i8;

                    let prev_rank= match &device.rank_info.last_rank {
                        Some(last_rank) => last_rank.rank as i8,
                        None => curr_rank as i8
                    };

                    let curr_rating_ordinal = value.rating_ordinal;

                    if prev_rating_ordinal == 0.0 {
                        prev_rating_ordinal = curr_rating_ordinal;
                    }

                    Box::new(RustRankInfo {
                        rank: curr_rank as c_uchar,
                        rating_ordinal: value.rating_ordinal,
                        global_placing: value.global_placing,
                        regional_placing: value.regional_placing,
                        rating_update_count: value.rating_update_count,
                        rating_change: curr_rating_ordinal - prev_rating_ordinal,
                        rank_change: curr_rank - prev_rank,
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
        tracing::info!(target: Log::SlippiOnline, "{}", rank_info.rank);

        Box::into_raw(rank_info)
    })
}