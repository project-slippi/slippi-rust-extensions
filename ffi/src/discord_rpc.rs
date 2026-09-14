use std::ffi::c_char;

use slippi_exi_device::{DiscordRpcConfiguration, SlippiEXIDevice};

use crate::c_str_to_string;

/// Configures Discord Rich Presence. `show_rank` is the player's in-game
/// rank-display preference; when false, the presence omits the rank badge.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_exi_device_configure_discord_rpc(exi_device_instance_ptr: usize, is_enabled: bool, show_rank: bool) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let mut device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let discord_rpc_config = match is_enabled {
        true => DiscordRpcConfiguration::Start { show_rank },
        false => DiscordRpcConfiguration::Stop,
    };
    device.configure_discord_rpc(discord_rpc_config);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Pushes the current Slippi matchmaking state to Discord Rich Presence. The
/// Rust side edge-detects and only re-renders when something changes.
/// `opponent_name` may be null or empty when unknown; `opponent_rank` is
/// negative when unknown.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_exi_device_update_matchmaking_state(
    exi_device_instance_ptr: usize,
    process_state: u8,
    online_mode: u8,
    opponent_name: *const c_char,
    opponent_rank: i8,
) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let fn_name = "slprs_exi_device_update_matchmaking_state";
    let opponent_name = match opponent_name.is_null() {
        true => String::new(),
        false => c_str_to_string(opponent_name, fn_name, "opponent_name"),
    };

    device.update_matchmaking_state(process_state, online_mode, opponent_name, opponent_rank);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}

/// Pushes the current Melee scene and character-select state to Discord Rich
/// Presence, mirroring how matchmaking state is pushed. `css_char_ids` points
/// to four external Melee character IDs (one per port); `0xFF` marks an empty
/// port. `local_port` is the 0-based port of the local player. The Rust side
/// edge-detects and only re-renders when something changes.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_exi_device_update_scene_state(
    exi_device_instance_ptr: usize,
    major_scene: u8,
    minor_scene: u8,
    css_char_ids: *const u8,
    local_port: u8,
    stage_id: u8,
) {
    // Coerce the instance from the pointer. This is theoretically safe since we control
    // the C++ side and can guarantee that the `exi_device_instance_ptr` is only owned
    // by the C++ EXI device, and is created/destroyed with the corresponding lifetimes.
    let device = unsafe { Box::from_raw(exi_device_instance_ptr as *mut SlippiEXIDevice) };

    let char_ids = match css_char_ids.is_null() {
        true => [0xFF; 4],
        false => {
            // The C++ side always passes a fixed four-byte, port-indexed array.
            let slice = unsafe { std::slice::from_raw_parts(css_char_ids, 4) };
            [slice[0], slice[1], slice[2], slice[3]]
        },
    };

    device.update_scene_state(major_scene, minor_scene, char_ids, local_port, stage_id);

    // Fall back into a raw pointer so Rust doesn't obliterate the object.
    let _leak = Box::into_raw(device);
}
