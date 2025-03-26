use std::ffi::c_char;

use std::sync::Arc;
use std::sync::Mutex;

//use slippi_rank_info::{};

use crate::{c_str_to_string, with};

/// Creates a new Player Report and leaks it, returning the pointer.
///
/// This should be passed on to a GameReport for processing.
#[no_mangle]
pub extern "C" fn slprs_player_get_rank() -> usize {
    let uid = c_str_to_string(uid, "slprs_player_get_rank", "uid");

    let report_instance_ptr = Box::into_raw(report) as usize;

    report_instance_ptr
}