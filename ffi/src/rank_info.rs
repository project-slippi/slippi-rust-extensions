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

            // let connect_code_str = user.connect_code.as_str();
            let connect_code_str = "MAGIC#1";
            let res = RankManager::fetch_user_rank(&device.rank_info, connect_code_str);

            match res {
                Ok(value) => {
                    tracing::info!(target: Log::SlippiOnline, "{}", value.rating_ordinal);
                    Box::new(RustRankInfo {
                        rank: RankManager::decide_rank(
                            value.rating_ordinal, 
                            value.daily_global_placement.unwrap_or_default(), 
                            value.daily_regional_placement.unwrap_or_default(), 
                            value.rating_update_count
                        ) as u8,
                        rating_ordinal: value.rating_ordinal,
                        global_placing: value.daily_global_placement.unwrap_or_default(),
                        regional_placing: value.daily_regional_placement.unwrap_or_default(),
                        rating_update_count: value.rating_update_count,
                        // TODO :: add decide_rank_change or something to handle this, also save last resp
                        rating_change: 0.0,
                        rank_change: 0,
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