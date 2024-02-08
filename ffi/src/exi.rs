use std::ffi::c_char;

use dolphin_integrations::Log;
use slippi_exi_device::{Config, FilePathsConfig, JukeboxConfiguration, SCMConfig, SlippiEXIDevice};
use slippi_game_reporter::GameReport;

use crate::c_str_to_string;

/// A configuration struct for passing over certain argument types from the C/C++ side.
///
/// The number of arguments necessary to shuttle across the FFI boundary when starting the
/// EXI device is higher than ideal at the moment, though it should lessen with time. For now,
/// this struct exists to act as a slightly more sane approach to readability of the args
/// structure.
#[repr(C)]
pub struct SlippiRustEXIConfig {
    // Paths
    pub iso_path: *const c_char,
    pub user_json_path: *const c_char,

    // Git version number
    pub scm_slippi_semver_str: *const c_char,

    // We don't currently need the below, but they're stubbed in case anyone ends up
    // needing to add 'em.
    //
    // pub scm_desc_str: *const c_char,
    // pub scm_branch_str: *const c_char,
    // pub scm_rev_str: *const c_char,
    // pub scm_rev_git_str: *const c_char,
    // pub scm_rev_cache_str: *const c_char,
    // pub netplay_dolphin_ver: *const c_char,
    // pub scm_distributor_str: *const c_char,

    // Hooks
    pub osd_add_msg_fn: unsafe extern "C" fn(*const c_char, u32, u32),
}

/// Creates and leaks a shadow EXI device with the provided configuration.
///
/// The C++ (Dolphin) side of things should call this and pass the appropriate arguments. At
/// that point, everything on the Rust side is its own universe, and should be told to shut
/// down (at whatever point) via the corresponding `slprs_exi_device_destroy` function.
///
/// The returned pointer from this should *not* be used after calling `slprs_exi_device_destroy`.
#[no_mangle]
pub extern "C" fn slprs_exi_device_create(config: SlippiRustEXIConfig) -> usize {
    dolphin_integrations::ffi::osd::set_global_hook(config.osd_add_msg_fn);

    let fn_name = "slprs_exi_device_create";

    let exi_device = Box::new(SlippiEXIDevice::new(Config {
        paths: FilePathsConfig {
            iso: c_str_to_string(config.iso_path, fn_name, "iso_path"),
            user_json: c_str_to_string(config.user_json_path, fn_name, "user_json"),
        },

        scm: SCMConfig {
            slippi_semver: c_str_to_string(config.scm_slippi_semver_str, fn_name, "slippi_semver"),
        },
    }));

    let exi_device_instance_ptr = Box::into_raw(exi_device) as usize;

    tracing::warn!(
        target: Log::SlippiOnline,
        ptr = exi_device_instance_ptr,
        "Initialized Rust EXI Device"
    );

    exi_device_instance_ptr
}

/// The C++ (Dolphin) side of things should call this to notify the Rust side that it
/// can safely shut down and clean up.
#[no_mangle]
pub extern "C" fn slprs_exi_device_destroy(exi_device_instance_ptr: usize) {
    tracing::warn!(
        target: Log::SlippiOnline,
        ptr = exi_device_instance_ptr,
        "Destroying Rust EXI Device"
    );

    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    unsafe {
        // Coerce ownership back, then let standard Drop semantics apply
        let _device = Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice);
    }
}

/// This method is for the C++ side to notify that the Memory system is initialized and ready
/// for use; the EXI device can then initialize any systems it needs that rely on the offset.
#[no_mangle]
pub extern "C" fn slprs_exi_device_on_memory_initialized(exi_device_instance_ptr: usize, m_p_ram: *const u8) {
    let offset = m_p_ram as usize;

    tracing::warn!(target: Log::SlippiOnline, ptr = exi_device_instance_ptr, m_pRAM = offset);

    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.on_memory_initialized(offset);

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);
}

/// This method should be called from the EXI device subclass shim that's registered on
/// the Dolphin side, corresponding to:
///
/// `virtual void DMAWrite(u32 _uAddr, u32 _uSize);`
#[no_mangle]
pub extern "C" fn slprs_exi_device_dma_write(exi_device_instance_ptr: usize, address: *const u8, size: *const u8) {
    // Coerce the instance back from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.dma_write(address as usize, size as usize);

    // Fall back into a raw pointer so Rust doesn't obliterate the object
    let _leak = Box::into_raw(device);
}

/// This method should be called from the EXI device subclass shim that's registered on
/// the Dolphin side, corresponding to:
///
/// `virtual void DMARead(u32 _uAddr, u32 _uSize);`
#[no_mangle]
pub extern "C" fn slprs_exi_device_dma_read(exi_device_instance_ptr: usize, address: *const u8, size: *const u8) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` pointer is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    device.dma_read(address as usize, size as usize);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Moves ownership of the `GameReport` at the specified address to the
/// `SlippiGameReporter` on the EXI Device the corresponding address. This
/// will then add it to the processing pipeline.
///
/// The reporter will manage the actual... reporting.
#[no_mangle]
pub extern "C" fn slprs_exi_device_log_game_report(instance_ptr: usize, game_report_instance_ptr: usize) {
    // Coerce the instances from the pointers. This is theoretically safe since we control
    // the C++ side and can guarantee that the pointers are only owned
    // by us, and are created/destroyed with the corresponding lifetimes.
    let (mut device, game_report) = unsafe {
        (
            Box::from_raw(instance_ptr as *mut SlippiEXIDevice),
            Box::from_raw(game_report_instance_ptr as *mut GameReport),
        )
    };

    device.game_reporter.log_report(*game_report);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to `SlippiGameReporter::start_new_session`.
#[no_mangle]
pub extern "C" fn slprs_exi_device_start_new_reporter_session(instance_ptr: usize) {
    // Coerce the instances from the pointers. This is theoretically safe since we control
    // the C++ side and can guarantee that the pointers are only owned
    // by us, and are created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(instance_ptr as *mut SlippiEXIDevice) };

    device.game_reporter.start_new_session();

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to the `SlippiGameReporter` on the EXI device to report a
/// match completion event.
#[no_mangle]
pub extern "C" fn slprs_exi_device_report_match_completion(instance_ptr: usize, match_id: *const c_char, end_mode: u8) {
    // Coerce the instances from the pointers. This is theoretically safe since we control
    // the C++ side and can guarantee that the pointers are only owned
    // by us, and are created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(instance_ptr as *mut SlippiEXIDevice) };

    let fn_name = "slprs_exi_device_report_match_completion";
    let match_id = c_str_to_string(match_id, fn_name, "match_id");

    device.game_reporter.report_completion(match_id, end_mode);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to the `SlippiGameReporter` on the EXI device to report a
/// match abandon event.
#[no_mangle]
pub extern "C" fn slprs_exi_device_report_match_abandonment(instance_ptr: usize, match_id: *const c_char) {
    // Coerce the instances from the pointers. This is theoretically safe since we control
    // the C++ side and can guarantee that the pointers are only owned
    // by us, and are created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(instance_ptr as *mut SlippiEXIDevice) };

    let fn_name = "slprs_exi_device_report_match_abandonment";
    let match_id = c_str_to_string(match_id, fn_name, "match_id");

    device.game_reporter.report_abandonment(match_id);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to `SlippiGameReporter::push_replay_data`.
#[no_mangle]
pub extern "C" fn slprs_exi_device_reporter_push_replay_data(instance_ptr: usize, data: *const u8, length: u32) {
    // Convert our pointer to a Rust slice so that the game reporter
    // doesn't need to deal with anything C-ish.
    let slice = unsafe { std::slice::from_raw_parts(data, length as usize) };

    // Coerce the instances from the pointers. This is theoretically safe since we control
    // the C++ side and can guarantee that the pointers are only owned
    // by us, and are created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(instance_ptr as *mut SlippiEXIDevice) };

    device.game_reporter.push_replay_data(slice);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Configures the Jukebox process. This needs to be called after the EXI device is created
/// in order for certain pieces of Dolphin to be properly initalized; this may change down
/// the road though and is not set in stone.
#[no_mangle]
pub extern "C" fn slprs_exi_device_configure_jukebox(
    exi_device_instance_ptr: usize,
    is_enabled: bool,
    initial_dolphin_system_volume: u8,
    initial_dolphin_music_volume: u8,
) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let jukebox_config = match is_enabled {
        true => JukeboxConfiguration::Start {
            initial_dolphin_system_volume,
            initial_dolphin_music_volume,
        },
        false => JukeboxConfiguration::Stop,
    };
    device.configure_jukebox(jukebox_config);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

#[no_mangle]
pub extern "C" fn slprs_start_discord_rich_presence(exi_device_instance_ptr: usize, m_p_ram: *const u8) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let m_p_ram = m_p_ram as usize;
    let config = slippi_exi_device::DiscordActivityHandlerConfiguration::Start { m_p_ram };
    device.configure_discord_handler(config);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}
