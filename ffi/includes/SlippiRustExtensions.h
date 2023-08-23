#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>

/// This enum is duplicated from `slippi_game_reporter::OnlinePlayMode` in order
/// to appease cbindgen, which cannot see the type from the other module for
/// inspection.
///
/// This enum will likely go away as things move towards Rust, since it's effectively
/// just C FFI glue code.
enum SlippiMatchmakingOnlinePlayMode {
  Ranked = 0,
  Unranked = 1,
  Direct = 2,
  Teams = 3,
};

/// A configuration struct for passing over certain argument types from the C/C++ side.
///
/// The number of arguments necessary to shuttle across the FFI boundary when starting the
/// EXI device is higher than ideal at the moment, though it should lessen with time. For now,
/// this struct exists to act as a slightly more sane approach to readability of the args
/// structure.
struct SlippiRustEXIConfig {
  const char *iso_path;
  const char *user_json_path;
  const char *scm_slippi_semver_str;
  void (*osd_add_msg_fn)(const char*, uint32_t, uint32_t);
};

/// An intermediary type for moving `UserInfo` across the FFI boundary.
///
/// This type is C compatible, and we coerce Rust types into C types for this struct to
/// ease passing things over. This must be free'd on the Rust side via `slprs_user_free_info`.
struct RustUserInfo {
  const char *uid;
  const char *play_key;
  const char *display_name;
  const char *connect_code;
  const char *latest_version;
};

/// An intermediary type for moving chat messages across the FFI boundary.
///
/// This type is C compatible, and we coerce Rust types into C types for this struct to
/// ease passing things over. This must be free'd on the Rust side via `slprs_user_free_messages`.
struct RustChatMessages {
  char **data;
  int len;
};

extern "C" {

/// Creates and leaks a shadow EXI device with the provided configuration.
///
/// The C++ (Dolphin) side of things should call this and pass the appropriate arguments. At
/// that point, everything on the Rust side is its own universe, and should be told to shut
/// down (at whatever point) via the corresponding `slprs_exi_device_destroy` function.
///
/// The returned pointer from this should *not* be used after calling `slprs_exi_device_destroy`.
uintptr_t slprs_exi_device_create(SlippiRustEXIConfig config);

/// The C++ (Dolphin) side of things should call this to notify the Rust side that it
/// can safely shut down and clean up.
void slprs_exi_device_destroy(uintptr_t exi_device_instance_ptr);

/// This method should be called from the EXI device subclass shim that's registered on
/// the Dolphin side, corresponding to:
///
/// `virtual void DMAWrite(u32 _uAddr, u32 _uSize);`
void slprs_exi_device_dma_write(uintptr_t exi_device_instance_ptr,
                                const uint8_t *address,
                                const uint8_t *size);

/// This method should be called from the EXI device subclass shim that's registered on
/// the Dolphin side, corresponding to:
///
/// `virtual void DMARead(u32 _uAddr, u32 _uSize);`
void slprs_exi_device_dma_read(uintptr_t exi_device_instance_ptr,
                               const uint8_t *address,
                               const uint8_t *size);

/// Moves ownership of the `GameReport` at the specified address to the
/// `SlippiGameReporter` on the EXI Device the corresponding address. This
/// will then add it to the processing pipeline.
///
/// The reporter will manage the actual... reporting.
void slprs_exi_device_log_game_report(uintptr_t instance_ptr, uintptr_t game_report_instance_ptr);

/// Calls through to `SlippiGameReporter::start_new_session`.
void slprs_exi_device_start_new_reporter_session(uintptr_t instance_ptr);

/// Calls through to the `SlippiGameReporter` on the EXI device to report a
/// match completion event.
void slprs_exi_device_report_match_completion(uintptr_t instance_ptr,
                                              const char *match_id,
                                              uint8_t end_mode);

/// Calls through to the `SlippiGameReporter` on the EXI device to report a
/// match abandon event.
void slprs_exi_device_report_match_abandonment(uintptr_t instance_ptr, const char *match_id);

/// Calls through to `SlippiGameReporter::push_replay_data`.
void slprs_exi_device_reporter_push_replay_data(uintptr_t instance_ptr,
                                                const uint8_t *data,
                                                uint32_t length);

/// Configures the Jukebox process. This needs to be called after the EXI device is created
/// in order for certain pieces of Dolphin to be properly initalized; this may change down
/// the road though and is not set in stone.
void slprs_exi_device_configure_jukebox(uintptr_t exi_device_instance_ptr,
                                        bool is_enabled,
                                        uint8_t initial_dolphin_system_volume,
                                        uint8_t initial_dolphin_music_volume);

/// Creates a new Player Report and leaks it, returning the pointer.
///
/// This should be passed on to a GameReport for processing.
uintptr_t slprs_player_report_create(const char *uid,
                                     uint8_t slot_type,
                                     double damage_done,
                                     uint8_t stocks_remaining,
                                     uint8_t character_id,
                                     uint8_t color_id,
                                     int64_t starting_stocks,
                                     int64_t starting_percent);

/// Creates a new GameReport and leaks it, returning the instance pointer
/// after doing so.
///
/// This is expected to ultimately be passed to the game reporter, which will handle
/// destruction and cleanup.
uintptr_t slprs_game_report_create(const char *uid,
                                   const char *play_key,
                                   SlippiMatchmakingOnlinePlayMode online_mode,
                                   const char *match_id,
                                   uint32_t duration_frames,
                                   uint32_t game_index,
                                   uint32_t tie_break_index,
                                   int8_t winner_index,
                                   uint8_t game_end_method,
                                   int8_t lras_initiator,
                                   int32_t stage_id);

/// Takes ownership of the `PlayerReport` at the specified pointer, adding it to the
/// `GameReport` at the corresponding pointer.
void slprs_game_report_add_player_report(uintptr_t instance_ptr,
                                         uintptr_t player_report_instance_ptr);

/// Calls through to `Jukebox::start_song`.
void slprs_jukebox_start_song(uintptr_t exi_device_instance_ptr,
                              uint64_t hps_offset,
                              uintptr_t hps_length);

/// Calls through to `Jukebox::stop_music`.
void slprs_jukebox_stop_music(uintptr_t exi_device_instance_ptr);

/// Calls through to `Jukebox::set_volume` with the Melee volume control.
void slprs_jukebox_set_melee_music_volume(uintptr_t exi_device_instance_ptr, uint8_t volume);

/// Calls through to `Jukebox::set_volume` with the DolphinSystem volume control.
void slprs_jukebox_set_dolphin_system_volume(uintptr_t exi_device_instance_ptr, uint8_t volume);

/// Calls through to `Jukebox::set_volume` with the DolphinMusic volume control.
void slprs_jukebox_set_dolphin_music_volume(uintptr_t exi_device_instance_ptr, uint8_t volume);

/// This should be called from the Dolphin LogManager initialization to ensure that
/// all logging needs on the Rust side are configured appropriately.
///
/// For more information, consult `dolphin_logger::init`.
///
/// Note that `logger_fn` cannot be type-aliased here, otherwise cbindgen will
/// mess up the header output. That said, the function type represents:
///
/// ```
/// void Log(level, log_type, msg);
/// ```
void slprs_logging_init(void (*logger_fn)(int, int, const char*));

/// Registers a log container, which mirrors a Dolphin `LogContainer` (`RustLogContainer`).
///
/// See `dolphin_logger::register_container` for more information.
void slprs_logging_register_container(const char *kind,
                                      int log_type,
                                      bool is_enabled,
                                      int default_log_level);

/// Updates the configuration for a registered logging container.
///
/// For more information, see `dolphin_logger::update_container`.
void slprs_logging_update_container(const char *kind, bool enabled, int level);

/// Updates the configuration for registered logging container on mainline
///
/// For more information, see `dolphin_logger::update_container`.
void slprs_mainline_logging_update_log_level(int level);

/// Instructs the `UserManager` on the EXI Device at the provided pointer to attempt
/// authentication. This runs synchronously on whatever thread it's called on.
bool slprs_user_attempt_login(uintptr_t exi_device_instance_ptr);

/// Instructs the `UserManager` on the EXI Device at the provided pointer to try to
/// open the login page in a system-provided browser view.
void slprs_user_open_login_page(uintptr_t exi_device_instance_ptr);

/// Instructs the `UserManager` on the EXI Device at the provided pointer to attempt
/// to initiate the older update flow.
bool slprs_user_update_app(uintptr_t exi_device_instance_ptr);

/// Instructs the `UserManager` on the EXI Device at the provided pointer to start watching
/// for the presence of a `user.json` file. The `UserManager` should have the requisite path
/// already from EXI device instantiation.
void slprs_user_listen_for_login(uintptr_t exi_device_instance_ptr);

/// Instructs the `UserManager` on the EXI Device at the provided pointer to sign the user out.
/// This will delete the `user.json` file from the underlying filesystem.
void slprs_user_logout(uintptr_t exi_device_instance_ptr);

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to overwrite the
/// latest version field on the current user.
void slprs_user_overwrite_latest_version(uintptr_t exi_device_instance_ptr, const char *version);

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to determine
/// authentication status.
bool slprs_user_get_is_logged_in(uintptr_t exi_device_instance_ptr);

/// Hooks through the `UserManager` on the EXI Device at the provided pointer to get information
/// for the current user. This then wraps it in a C struct to pass back so that ownership is safely
/// moved.
///
/// This involves slightly more allocations than ideal, so this shouldn't be called in a hot path.
/// Over time this issue will not matter as once Matchmaking is moved to Rust we can share things
/// quite easily.
RustUserInfo *slprs_user_get_info(uintptr_t exi_device_instance_ptr);

/// Takes ownership back of a `UserInfo` struct and drops it.
///
/// When the C/C++ side grabs `UserInfo`, it needs to ensure that it's passed back to Rust
/// to ensure that the memory layout matches - do _not_ call `free` on `UserInfo`, pass it here
/// instead.
void slprs_user_free_info(RustUserInfo *ptr);

/// Returns a C-compatible struct containing the chat message options for the current user.
///
/// The return value of this _must_ be passed back to `slprs_user_free_messages` to free memory.
RustChatMessages *slprs_user_get_messages(uintptr_t exi_device_instance_ptr);

/// Returns a C-compatible struct containing the default chat message options.
///
/// The return value of this _must_ be passed back to `slprs_user_free_messages` to free memory.
RustChatMessages *slprs_user_get_default_messages(uintptr_t exi_device_instance_ptr);

/// Takes back ownership of a `RustChatMessages` instance and frees the underlying data
/// by converting it into the proper Rust types.
void slprs_user_free_messages(RustChatMessages *ptr);

} // extern "C"
