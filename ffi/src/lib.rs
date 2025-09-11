//! This library is the core interface for the Rust side of things, and consists
//! predominantly of C FFI bridging functions that can be called from the Dolphin
//! side of things.
//!
//! This library auto-generates C headers on build, and Slippi Dolphin is pre-configured
//! to locate these headers and link the entire dylib.

use std::ffi::{CStr, c_char};

use dolphin_integrations::Log;

pub mod exi;
pub mod game_reporter;
pub mod jukebox;
pub mod logger;
#[cfg(feature = "playback")]
pub mod playback;

pub mod rank_info;
pub mod user;

/// A small helper method for moving in and out of our known types.
///
/// > This method operates in `unsafe` territory as it's operating on pointers owned by the C++
/// > side. That said, this isn't really a "library" in the traditional sense - we control the C++
/// > side and can verify the few places where these pointers are passed over. We silo the `unsafe`
/// > usage here to not bloat the FFI layer while transitioning things; this should not be taken as
/// > an invitation to do all `unsafe` calls this way.
/// >
/// > This method has internal documentation surrounding safety assumptions to explain the
/// > reasoning further.
pub(crate) fn with<T, F>(instance_ptr: usize, handler: F)
where
    F: FnOnce(&mut T),
{
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `instance_ptr` is only owned
    // by us, and is created/destroyed with the corresponding lifetimes.
    let mut instance = unsafe { Box::from_raw(instance_ptr as *mut T) };

    handler(&mut instance);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(instance);
}

/// A small helper method for moving in and out of our known types.
///
/// This variant can be used to return a value from within a handler.
///
/// > This method operates in `unsafe` territory as it's operating on pointers owned by the C++
/// > side. That said, this isn't really a "library" in the traditional sense - we control the C++
/// > side and can verify the few places where these pointers are passed over. We silo the `unsafe`
/// > usage here to not bloat the FFI layer while transitioning things; this should not be taken as
/// > an invitation to do all `unsafe` calls this way.
/// >
/// > This method has internal documentation surrounding safety assumptions to explain the
/// > reasoning further.
pub(crate) fn with_returning<T, F, R>(instance_ptr: usize, handler: F) -> R
where
    F: FnOnce(&mut T) -> R,
{
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `instance_ptr` is only owned
    // by us, and is created/destroyed with the corresponding lifetimes.
    let mut instance = unsafe { Box::from_raw(instance_ptr as *mut T) };

    let ret = handler(&mut instance);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(instance);

    ret
}

/// A helper function for converting c str types to Rust ones with
/// some optional args for aiding in debugging should this ever be a problem.
///
/// This will panic if the strings being passed over cannot be converted. This is intentional, as
/// the entire application would be in an invalid state if this was not working.
///
/// > This method operates in `unsafe` territory as it's operating on pointers owned by the C++
/// > side. That said, this isn't really a "library" in the traditional sense - we control the C++
/// > side and can verify the few places where these pointers are passed over. We silo the `unsafe`
/// > usage here to not bloat the FFI layer while transitioning things; this should not be taken as
/// > an invitation to do all `unsafe` calls this way.
/// >
/// > This method has internal documentation surrounding safety assumptions to explain the
/// > reasoning further.
pub(crate) fn c_str_to_string(string: *const c_char, fn_label: &str, err_label: &str) -> String {
    // This is theoretically safe as we control the strings being passed from
    // the C++ side, and can mostly guarantee that we know what we're getting.
    let slice = unsafe { CStr::from_ptr(string) };

    match slice.to_str() {
        Ok(s) => s.to_string(),

        Err(e) => {
            tracing::error!(
                target: Log::SlippiOnline,
                error = ?e,
                "[{}] Failed to bridge {}, will panic",
                fn_label,
                err_label
            );

            panic!("Unable to bridge necessary type, panicing");
        },
    }
}
