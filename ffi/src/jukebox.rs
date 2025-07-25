use slippi_exi_device::SlippiEXIDevice;
use slippi_jukebox::VolumeControl;

/// Calls through to `Jukebox::start_song`.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_jukebox_start_song(exi_device_instance_ptr: usize, hps_offset: u64, hps_length: usize) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    if let Some(jukebox) = device.jukebox.as_mut() {
        jukebox.start_song(hps_offset, hps_length);
    }

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to `Jukebox::stop_music`.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_jukebox_stop_music(exi_device_instance_ptr: usize) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    if let Some(jukebox) = device.jukebox.as_mut() {
        jukebox.stop_music();
    }

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to `Jukebox::set_volume` with the Melee volume control.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_jukebox_set_melee_music_volume(exi_device_instance_ptr: usize, volume: u8) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    if let Some(jukebox) = device.jukebox.as_mut() {
        jukebox.set_volume(VolumeControl::Melee, volume);
    }

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to `Jukebox::set_volume` with the DolphinSystem volume control.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_jukebox_set_dolphin_system_volume(exi_device_instance_ptr: usize, volume: u8) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    if let Some(jukebox) = device.jukebox.as_mut() {
        jukebox.set_volume(VolumeControl::DolphinSystem, volume);
    }

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Calls through to `Jukebox::set_volume` with the DolphinMusic volume control.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_jukebox_set_dolphin_music_volume(exi_device_instance_ptr: usize, volume: u8) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    if let Some(jukebox) = device.jukebox.as_mut() {
        jukebox.set_volume(VolumeControl::DolphinMusic, volume);
    }

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}
