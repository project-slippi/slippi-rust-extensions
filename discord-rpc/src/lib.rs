use std::convert::TryInto;
use std::fs::File;
use std::ops::ControlFlow::{self, Break, Continue};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{thread::sleep, time::Duration};

use dolphin_integrations::{Color, Dolphin, Duration as OSDDuration, Log};
use hps_decode::{Hps, PcmIterator};
use process_memory::{LocalMember, Memory};
use rodio::{OutputStream, Sink};

mod errors;
pub use errors::DiscordRPCError;
use DiscordRPCError::*;

mod scenes;
use scenes::scene_ids::*;


mod utils;

pub(crate) type Result<T> = std::result::Result<T, DiscordRPCError>;

/// Represents a foreign method from the Dolphin side for grabbing the current volume.
/// Dolphin represents this as a number from 0 - 100; 0 being mute.
pub type ForeignGetVolumeFn = unsafe extern "C" fn() -> std::ffi::c_int;

const THREAD_LOOP_SLEEP_TIME_MS: u64 = 30;
const CHILD_THREAD_COUNT: usize = 2;

/// By default Slippi DiscordRPC plays music slightly louder than vanilla melee
/// does. This reduces the overall music volume output to 80%. Not totally sure
/// if that's the correct amount, but it sounds about right.
const VOLUME_REDUCTION_MULTIPLIER: f32 = 0.8;

#[derive(Debug, PartialEq)]
struct DolphinGameState {
    in_game: bool,
    in_menus: bool,
    scene_major: u8,
    scene_minor: u8,
    stage_id: u8,
    volume: f32,
    is_paused: bool,
    match_info: u8,
}

impl Default for DolphinGameState {
    fn default() -> Self {
        Self {
            in_game: false,
            in_menus: false,
            scene_major: SCENE_MAIN_MENU,
            scene_minor: 0,
            stage_id: 0,
            volume: 0.0,
            is_paused: false,
            match_info: 0,
        }
    }
}

#[derive(Debug)]
enum MeleeEvent {
    TitleScreenEntered,
    MenuEntered,
    LotteryEntered,
    GameStart(u8), // stage id
    GameEnd,
    RankedStageStrikeEntered,
    VsOnlineOpponent,
    Pause,
    Unpause,
    SetVolume(f32),
    NoOp,
}

#[derive(Debug, Clone)]
enum DiscordRPCEvent {
    Dropped,
}

#[derive(Debug)]
pub struct DiscordActivityHandler {
    channel_senders: [Sender<DiscordRPCEvent>; CHILD_THREAD_COUNT],
}

impl DiscordActivityHandler {
    /// Returns a DiscordRPC instance that will immediately spawn two child threads
    /// to try and read game memory and play music. When the returned instance is
    /// dropped, the child threads will terminate and the music will stop.
    pub fn new(m_p_ram: usize, iso_path: String, get_dolphin_volume_fn: ForeignGetVolumeFn) -> Result<Self>  {
        tracing::info!(target: Log::DiscordRPC, "Initializing Slippi Discord RPC");

        // We are implicitly trusting that these pointers will outlive the jukebox instance
        let get_dolphin_volume = move || unsafe { get_dolphin_volume_fn() } as f32 / 100.0;

        // This channel is used for the `DiscordRPCMessageDispatcher` thread to send
        // messages to the `DiscordRPCMusicPlayer` thread
        let (melee_event_tx, melee_event_rx) = channel::<MeleeEvent>();

        // These channels allow the jukebox instance to notify both child
        // threads when something important happens. Currently its only purpose
        // is to notify them that the instance is about to be dropped so they
        // should terminate
        let (message_dispatcher_thread_tx, message_dispatcher_thread_rx) = channel::<DiscordRPCEvent>();
        let (music_thread_tx, music_thread_rx) = channel::<DiscordRPCEvent>();

        // Spawn message dispatcher thread
        std::thread::Builder::new()
            .name("DiscordRPCMessageDispatcher".to_string())
            .spawn(move || {
                match Self::dispatch_messages(m_p_ram, get_dolphin_volume, message_dispatcher_thread_rx, melee_event_tx) {
                    Err(e) => tracing::error!(
                        target: Log::DiscordRPC,
                        error = ?e,
                        "DiscordRPCMessageDispatcher thread encountered an error: {e}"
                    ),
                    _ => (),
                }
            })
            .map_err(ThreadSpawn)?;

        
    }

    /// This thread continuously reads select values from game memory as well
    /// as the current `volume` value in the dolphin configuration. If it
    /// notices anything change, it will dispatch a message to the
    /// `DiscordRPCMusicPlayer` thread.
    fn dispatch_messages(
        m_p_ram: usize,
        get_dolphin_volume: impl Fn() -> f32,
        message_dispatcher_thread_rx: Receiver<DiscordRPCEvent>,
        melee_event_tx: Sender<MeleeEvent>,
    ) -> Result<()> {
        // Initial "dolphin state" that will get updated over time
        let mut prev_state = DolphinGameState::default();

        loop {
            // Stop the thread if the jukebox instance will be been dropped
            if let Ok(event) = message_dispatcher_thread_rx.try_recv() {
                if matches!(event, DiscordRPCEvent::Dropped) {
                    return Ok(());
                }
            }

            // Continuously check if the dolphin state has changed
            let state = Self::read_dolphin_game_state(&m_p_ram, get_dolphin_volume())?;

            // If the state has changed,
            if prev_state != state {
                // dispatch a message to the music player thread
                let event = Self::produce_melee_event(&prev_state, &state);
                tracing::info!(target: Log::DiscordRPC, "{:?}", event);

                melee_event_tx.send(event).ok();
                prev_state = state;
            }

            sleep(Duration::from_millis(THREAD_LOOP_SLEEP_TIME_MS));
        }
    }

    /// This thread listens for incoming messages from the
    /// `DiscordRPCMessageDispatcher` thread and handles music playback
    /// accordingly.
    

    /// Handle a events received in the audio playback thread, by changing tracks,
    /// adjusting volume etc.
    fn handle_melee_event(
        event: MeleeEvent,
        sink: &Sink,
        volume: &mut f32,
    ) -> ControlFlow<()> {
        use self::MeleeEvent::*;

        // TODO:
        // - Intro movie
        //
        // - classic vs screen
        // - classic victory screen
        // - classic game over screen
        // - classic credits
        // - classic "congratulations movie"
        // - Adventure mode field intro music

        match event {
            TitleScreenEntered | GameEnd => {
                
               NoOp;
            },
            MenuEntered => {
                
               NoOp;
            },
            LotteryEntered => {
                NoOp;
            },
            VsOnlineOpponent => {
                NoOp;
            },
            RankedStageStrikeEntered => {
                NoOp;
            },
            GameStart(stage_id) => {
               NoOp;
            },
            Pause => {
                sink.set_volume(*volume * 0.2);
                return Continue(());
            },
            Unpause => {
                sink.set_volume(*volume);
                return Continue(());
            },
            SetVolume(received_volume) => {
                sink.set_volume(received_volume);
                *volume = received_volume;
                return Continue(());
            },
            NoOp => {
                return Continue(());
            },
        };

        Break(())
    }

    /// Given the previous dolphin state and current dolphin state, produce an event
    fn produce_melee_event(prev_state: &DolphinGameState, state: &DolphinGameState) -> MeleeEvent {
        let vs_screen_1 = state.scene_major == SCENE_VS_ONLINE
            && prev_state.scene_minor != SCENE_VS_ONLINE_VERSUS
            && state.scene_minor == SCENE_VS_ONLINE_VERSUS;
        let vs_screen_2 = prev_state.scene_minor == SCENE_VS_ONLINE_VERSUS && state.stage_id == 0;
        let entered_vs_online_opponent_screen = vs_screen_1 || vs_screen_2;

        if state.scene_major == SCENE_VS_ONLINE
            && prev_state.scene_minor != SCENE_VS_ONLINE_RANKED
            && state.scene_minor == SCENE_VS_ONLINE_RANKED
        {
            MeleeEvent::RankedStageStrikeEntered
        } else if !prev_state.in_menus && state.in_menus {
            MeleeEvent::MenuEntered
        } else if prev_state.scene_major != SCENE_TITLE_SCREEN && state.scene_major == SCENE_TITLE_SCREEN {
            MeleeEvent::TitleScreenEntered
        } else if entered_vs_online_opponent_screen {
            MeleeEvent::VsOnlineOpponent
        } else if prev_state.scene_major != SCENE_TROPHY_LOTTERY && state.scene_major == SCENE_TROPHY_LOTTERY {
            MeleeEvent::LotteryEntered
        } else if (!prev_state.in_game && state.in_game) || prev_state.stage_id != state.stage_id {
            MeleeEvent::GameStart(state.stage_id)
        } else if prev_state.in_game && state.in_game && state.match_info == 1 {
            MeleeEvent::GameEnd
        } else if prev_state.volume != state.volume {
            MeleeEvent::SetVolume(state.volume)
        } else if !prev_state.is_paused && state.is_paused {
            MeleeEvent::Pause
        } else if prev_state.is_paused && !state.is_paused {
            MeleeEvent::Unpause
        } else {
            MeleeEvent::NoOp
        }
    }

    /// Create a `DolphinGameState` by reading Dolphin's memory
    fn read_dolphin_game_state(m_p_ram: &usize, dolphin_volume_percent: f32) -> Result<DolphinGameState> {
        #[inline(always)]
        fn read<T: Copy>(offset: usize) -> Result<T> {
            Ok(unsafe { LocalMember::<T>::new_offset(vec![offset]).read().map_err(DolphinMemoryRead)? })
        }
        // https://github.com/bkacjios/m-overlay/blob/d8c629d/source/modules/games/GALE01-2.lua#L8
        let melee_volume_percent = ((read::<i8>(m_p_ram + 0x45C384)? as f32 - 100.0) * -1.0) / 100.0;
        // https://github.com/bkacjios/m-overlay/blob/d8c629d/source/modules/games/GALE01-2.lua#L16
        let scene_major = read::<u8>(m_p_ram + 0x479D30)?;
        // https://github.com/bkacjios/m-overlay/blob/d8c629d/source/modules/games/GALE01-2.lua#L19
        let scene_minor = read::<u8>(m_p_ram + 0x479D33)?;
        // https://github.com/bkacjios/m-overlay/blob/d8c629d/source/modules/games/GALE01-2.lua#L357
        let stage_id = read::<u8>(m_p_ram + 0x49E753)?;
        // https://github.com/bkacjios/m-overlay/blob/d8c629d/source/modules/games/GALE01-2.lua#L248
        // 0 = in game, 1 = GAME! screen, 2 = Stage clear in 1p mode? (maybe also victory screen), 3 = menu
        let match_info = read::<u8>(m_p_ram + 0x46B6A0)?;
        // https://github.com/bkacjios/m-overlay/blob/d8c629d/source/modules/games/GALE01-2.lua#L353
        let is_paused = read::<u8>(m_p_ram + 0x4D640F)? == 1;

        Ok(DolphinGameState {
            in_game: utils::is_in_game(scene_major, scene_minor),
            in_menus: utils::is_in_menus(scene_major, scene_minor),
            scene_major,
            scene_minor,
            volume: dolphin_volume_percent * melee_volume_percent * VOLUME_REDUCTION_MULTIPLIER,
            stage_id,
            is_paused,
            match_info,
        })
    }
}

impl Drop for DiscordActivityHandler {
    fn drop(&mut self) {
        tracing::info!(target: Log::DiscordRPC, "Dropping Slippi DiscordActivityHandler");
        for sender in &self.channel_senders {
            if let Err(e) = sender.send(DiscordRPCEvent::Dropped) {
                tracing::warn!(
                    target: Log::DiscordRPC,
                    "Failed to notify child thread that DiscordActivityHandler is dropping: {e}"
                );
            }
        }
    }
}
