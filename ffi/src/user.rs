use std::ffi::{c_char, c_int, CString};

use slippi_exi_device::SlippiEXIDevice;

use crate::{c_str_to_string, with, with_returning};

/// Instructs the `UserManager` on the EXI Device at the provided pointer to attempt
/// authentication. This runs synchronously on whatever thread it's called on.
#[no_mangle]
pub extern "C" fn slprs_user_attempt_login(exi_device_instance_ptr: usize) -> bool {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| device.user_manager.attempt_login())
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to try to
/// open the login page in a system-provided browser view.
#[no_mangle]
pub extern "C" fn slprs_user_open_login_page(exi_device_instance_ptr: usize) {
    with::<SlippiEXIDevice, _>(exi_device_instance_ptr, |device| {
        device.user_manager.open_login_page();
    });
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to attempt
/// to initiate the older update flow.
#[no_mangle]
pub extern "C" fn slprs_user_update_app(exi_device_instance_ptr: usize) -> bool {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| device.user_manager.update_app())
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to start watching
/// for the presence of a `user.json` file. The `UserManager` should have the requisite path
/// already from EXI device instantiation.
#[no_mangle]
pub extern "C" fn slprs_user_listen_for_login(exi_device_instance_ptr: usize) {
    with::<SlippiEXIDevice, _>(exi_device_instance_ptr, |device| {
        device.user_manager.watch_for_login();
    });
}

/// Instructs the `UserManager` on the EXI Device at the provided pointer to sign the user out.
/// This will delete the `user.json` file from the underlying filesystem.
#[no_mangle]
pub extern "C" fn slprs_user_logout(exi_device_instance_ptr: usize) {
    with::<SlippiEXIDevice, _>(exi_device_instance_ptr, |device| {
        device.user_manager.logout();
    });
}

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to overwrite the
/// latest version field on the current user.
#[no_mangle]
pub extern "C" fn slprs_user_overwrite_latest_version(exi_device_instance_ptr: usize, version: *const c_char) {
    let version = c_str_to_string(version, "slprs_user_overwrite_latest_version", "version");

    with::<SlippiEXIDevice, _>(exi_device_instance_ptr, move |device| {
        device.user_manager.overwrite_latest_version(version);
    });
}

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to determine
/// authentication status.
#[no_mangle]
pub extern "C" fn slprs_user_get_is_logged_in(exi_device_instance_ptr: usize) -> bool {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| device.user_manager.is_logged_in())
}

/// An intermediary type for moving `UserInfo` across the FFI boundary.
///
/// This type is C compatible, and we coerce Rust types into C types for this struct to
/// ease passing things over. This must be free'd on the Rust side via `slprs_user_free_info`.
#[repr(C)]
pub struct RustUserInfo {
    pub uid: *const c_char,
    pub play_key: *const c_char,
    pub display_name: *const c_char,
    pub connect_code: *const c_char,
    pub latest_version: *const c_char,
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
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let user_info = device.user_manager.get(|user| {
            let uid = CString::new(user.uid.as_str()).expect("uid CString failed").into_raw();

            let play_key = CString::new(user.play_key.as_str())
                .expect("play_key CString failed")
                .into_raw();

            let display_name = CString::new(user.display_name.as_str())
                .expect("display_name CString failed")
                .into_raw();

            let connect_code = CString::new(user.connect_code.as_str())
                .expect("connect_code CString failed")
                .into_raw();

            let latest_version = CString::new(user.latest_version.as_str())
                .expect("latest_version CString failed")
                .into_raw();

            Box::new(RustUserInfo {
                uid,
                play_key,
                display_name,
                connect_code,
                latest_version,
            })
        });

        Box::into_raw(user_info)
    })
}

/// Takes ownership back of a `UserInfo` struct and drops it.
///
/// When the C/C++ side grabs `UserInfo`, it needs to ensure that it's passed back to Rust
/// to ensure that the memory layout matches - do _not_ call `free` on `UserInfo`, pass it here
/// instead.
#[no_mangle]
pub extern "C" fn slprs_user_free_info(ptr: *mut RustUserInfo) {
    if ptr.is_null() {
        // Log here~?
        return;
    }

    // Unwrap a slew of pointers and let Rust drop everything accordingly.
    unsafe {
        let user_info = Box::from_raw(ptr);

        let _uid = CString::from_raw(user_info.uid as *mut _);
        let _play_key = CString::from_raw(user_info.play_key as *mut _);
        let _display_name = CString::from_raw(user_info.display_name as *mut _);
        let _connect_code = CString::from_raw(user_info.connect_code as *mut _);
        let _latest_version = CString::from_raw(user_info.latest_version as *mut _);
    }
}

/// An intermediary type for moving chat messages across the FFI boundary.
///
/// This type is C compatible, and we coerce Rust types into C types for this struct to
/// ease passing things over. This must be free'd on the Rust side via `slprs_user_free_messages`.
#[repr(C)]
pub struct RustChatMessages {
    pub data: *mut *mut c_char,
    pub len: c_int,
}

impl RustChatMessages {
    /// Common logic for taking a list of chat messages and converting them
    /// into an exportable C-type.
    ///
    /// This takes any type `S` that implements allowing a reference to the underlying bytes, which
    /// enables us to save some allocations in the case of default messages (static strs).
    fn from<S>(messages: &[S]) -> Self
    where
        S: AsRef<[u8]>,
    {
        // To move an array of C strings back, we'll create a Vec of CString pointers, shrink it,
        // and stash the len and pointer on the struct we're returning. The C++ side can unravel
        // as necessary, and the free method in this module should handle cleaning this up.
        let mut chat_messages: Vec<*mut _> = messages
            .iter()
            .map(|message| {
                CString::new(message.as_ref())
                    .expect("Unable to create CString for chat message")
                    .into_raw()
            })
            .collect();

        chat_messages.shrink_to_fit();

        let len = chat_messages.len() as c_int;
        let data = chat_messages.as_mut_ptr();
        std::mem::forget(chat_messages);

        Self { data, len }
    }
}

/// Returns a C-compatible struct containing the chat message options for the current user.
///
/// The return value of this _must_ be passed back to `slprs_user_free_messages` to free memory.
#[no_mangle]
pub extern "C" fn slprs_user_get_messages(exi_device_instance_ptr: usize) -> *mut RustChatMessages {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |device| {
        let messages = device.user_manager.get(|user| {
            Box::new(RustChatMessages::from(match &user.chat_messages {
                Some(messages) => messages,
                None => &[],
            }))
        });

        Box::into_raw(messages)
    })
}

/// Returns a C-compatible struct containing the default chat message options.
///
/// The return value of this _must_ be passed back to `slprs_user_free_messages` to free memory.
#[no_mangle]
pub extern "C" fn slprs_user_get_default_messages(exi_device_instance_ptr: usize) -> *mut RustChatMessages {
    with_returning::<SlippiEXIDevice, _, _>(exi_device_instance_ptr, |_device| {
        let messages = Box::new(RustChatMessages::from(&slippi_user::DEFAULT_CHAT_MESSAGES));
        Box::into_raw(messages)
    })
}

/// Takes back ownership of a `RustChatMessages` instance and frees the underlying data
/// by converting it into the proper Rust types.
#[no_mangle]
pub extern "C" fn slprs_user_free_messages(ptr: *mut RustChatMessages) {
    if ptr.is_null() {
        // Log here~?
        return;
    }

    unsafe {
        let messages = Box::from_raw(ptr);

        // Rebuild the Vec~
        let len = messages.len as usize;
        let messages = Vec::from_raw_parts(messages.data, len, len);

        // Consume, walk, and free the inner items
        for message in messages.into_iter() {
            let _message = CString::from_raw(message);
        }
    }
}
