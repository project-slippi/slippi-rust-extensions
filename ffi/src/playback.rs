// use std::ffi::{CString, c_char, c_int};

// use slippi_exi_device::SlippiEXIDevice;

// use crate::{c_str_to_string, with, with_returning};

// #[unsafe(no_mangle)]
// pub extern "C" fn slprs_is_replay_ready(exi_device_instance_ptr: usize) -> bool {
//     with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
//         // Get playback engine, if it is None, just return false
//         let Some(playback_engine) = device.playback else {
//             return false;
//         };
//     })
// }
