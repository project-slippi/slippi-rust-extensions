use std::ffi::{c_char, CString};

use dolphin_integrations::Log;
use slippi_exi_device::SlippiEXIDevice;

use crate::c_str_to_string;

/// Instructs the `UserManager` on the EXI Device at the provided pointer to attempt
/// authentication. This runs synchronously on whatever thread it's called on.
#[no_mangle]
pub extern "C" fn slprs_user_attempt_login(exi_device_instance_ptr: usize) -> bool {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let result = device.user_manager.attempt_login();

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);

    result
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to try to
/// open the login page in a system-provided browser view.
#[no_mangle]
pub extern "C" fn slprs_user_open_login_page(exi_device_instance_ptr: usize) {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.user_manager.open_login_page();

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to attempt
/// to initiate the older update flow.
#[no_mangle]
pub extern "C" fn slprs_user_update_app(exi_device_instance_ptr: usize) -> bool {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let result = device.user_manager.update_app();

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);

    result
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to start watching
/// for the presence of a `user.json` file. The `UserManager` should have the requisite path
/// already from EXI device instantiation.
#[no_mangle]
pub extern "C" fn slprs_user_listen_for_login(exi_device_instance_ptr: usize) {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.user_manager.watch_for_login();

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to sign the user out.
/// This will delete the `user.json` file from the underlying filesystem.
#[no_mangle]
pub extern "C" fn slprs_user_logout(exi_device_instance_ptr: usize) {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.user_manager.logout();

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);
}

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to overwrite the
/// latest version field on the current user.
#[no_mangle]
pub extern "C" fn slprs_user_overwrite_latest_version(
    exi_device_instance_ptr: usize,
    version: *const c_char
) {
    let version = c_str_to_string(version, "slprs_user_overwrite_latest_version", "version");

    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.user_manager.overwrite_latest_version(version);

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);
}

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to determine
/// authentication status.
#[no_mangle]
pub extern "C" fn slprs_user_get_is_logged_in(exi_device_instance_ptr: usize) -> bool {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let result = device.user_manager.is_logged_in();

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);

    result
}

/// An intermediary type for moving `UserInfo` across the FFI boundary.
///
/// This type is C compatible, and we coerce Rust types into C types for this struct to 
/// ease passing things over. 
#[repr(C)]
pub struct RustUserInfo {
    pub uid: *const c_char,
    pub play_key: *const c_char,
    pub display_name: *const c_char,
    pub connect_code: *const c_char,
    pub latest_version: *const c_char
}

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to get information
/// for the current user. This then wraps it in a C struct to pass back so that ownership is safely
/// moved.
///
/// This involves slightly more allocations than ideal, so this shouldn't be called in a hot path.
/// Over time this issue will not matter as once Matchmaking is moved to Rust we can share things
/// quite easily.
#[no_mangle]
pub extern "C" fn slprs_user_get_info(exi_device_instance_ptr: usize) -> *mut RustUserInfo {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let user_info = device.user_manager.get(|user| {
        Box::new(RustUserInfo {
            uid: CString::new(user.uid.as_str()).expect("Unable to create CString for uid").into_raw(),
            play_key: CString::new(user.play_key.as_str()).expect("Unable to create CString for play_key").into_raw(),
            display_name: CString::new(user.display_name.as_str()).expect("Unable to create CString for display_name").into_raw(),
            connect_code: CString::new(user.connect_code.as_str()).expect("Unable to create CString for connect_code").into_raw(),
            latest_version: CString::new(user.latest_version.as_str()).expect("Unable to create CString for latest_version").into_raw(),
        })
    });

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);

    Box::into_raw(user_info)
}

/// Takes ownership back of a `UserInfo` struct and drops it.
///
/// When the C/C++ side grabs `UserInfo`, it needs to ensure that it's passed back to Rust
/// to ensure that the memory layout matches - do _not_ call `free` on `UserInfo`, pass it here
/// instead.
#[no_mangle]
pub extern "C" fn slprs_user_free_info(ptr: *mut RustUserInfo) {
    if ptr.is_null() {
        // Log here~
        return;
    }

    unsafe {
        let _drop = Box::from_raw(ptr);
    }
}
