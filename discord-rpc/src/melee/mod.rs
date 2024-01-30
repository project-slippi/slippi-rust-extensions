use std::{fmt::Display};

use num_enum::TryFromPrimitive;
use strum::{IntoEnumIterator};
use strum_macros::{Display, EnumIter};
use tokio_util::sync::CancellationToken;

use crate::{discord::{DiscordClientRequest, DiscordClientRequestType, DiscordClientRequestTimestamp, DiscordClientRequestTimestampMode}, util::{current_unix_time, sleep}, melee::{stage::MeleeStage, character::MeleeCharacter}, config::{CONFIG}, tray::MeleeTrayEvent};

use self::{dolphin_mem::{DolphinMemory, util::R13}, msrb::MSRBOffset, multiman::MultiManVariant};

mod dolphin_mem;
mod msrb;
mod multiman;
pub mod stage;
pub mod character;
pub mod dolphin_user;

// reference: https://github.com/akaneia/m-ex/blob/master/MexTK/include/match.h#L11-L14
#[derive(PartialEq, EnumIter, Clone, Copy)]
enum TimerMode {
    Countup = 3,
    Countdown = 2,
    Hidden = 1,
    Frozen = 0,
}

#[derive(TryFromPrimitive, Display, Debug)]
#[repr(u8)]
enum MatchmakingMode {
    Idle = 0,
    Initializing = 1,
    Matchmaking = 2,
    OpponentConnecting = 3,
    ConnectionSuccess = 4,
    ErrorEncountered = 5
}

#[derive(Debug, TryFromPrimitive, Display, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum SlippiMenuScene {
    Ranked = 0,
    Unranked = 1,
    Direct = 2,
    Teams = 3
}

pub struct MeleeClient {
    mem: DolphinMemory,
    last_payload: DiscordClientRequest,
    last_tray_event: MeleeTrayEvent
}

#[derive(PartialEq, Clone, Copy,Debug)]
pub enum MeleeScene {
    MainMenu,
    VsMode,
    UnclePunch,
    TrainingMode,
    SlippiOnline(Option<SlippiMenuScene>),
    SlippiCss(Option<SlippiMenuScene>),
    HomeRunContest,
    TargetTest(Option<MeleeStage>),
    MultiManMelee(MultiManVariant)
}

impl Display for MeleeScene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::MainMenu => write!(f, "Main Menu"),
            Self::VsMode => write!(f, "Vs. Mode"),
            Self::UnclePunch => write!(f, "UnclePunch Training Mode"),
            Self::TrainingMode => write!(f, "Training Mode"),
            Self::SlippiOnline(Some(scene)) => write!(f, "{}", scene),
            Self::SlippiOnline(None) => write!(f, "Slippi Online"),
            Self::HomeRunContest => write!(f, "Home-Run Contest"),
            Self::TargetTest(stage_opt) => {
                if stage_opt.is_some() && CONFIG.with_ref(|c| c.stadium.btt.show_stage_name) {
                    write!(f, "{}", stage_opt.unwrap())
                } else {
                    write!(f, "Target Test")
                }
            },
            Self::MultiManMelee(variant) => write!(f, "Multi-Man Melee ({})", match variant {
                MultiManVariant::TenMan => "10 man",
                MultiManVariant::HundredMan => "100 man",
                MultiManVariant::ThreeMinute => "3 min",
                MultiManVariant::FifteenMinute => "15 min",
                MultiManVariant::Endless => "Endless",
                MultiManVariant::Cruel => "Cruel",
            }),
            Self::SlippiCss(_) => unimplemented!(),
        }
    }
}

impl MeleeClient {
    pub fn new() -> Self {
        MeleeClient { mem: DolphinMemory::new(), last_payload: DiscordClientRequest::clear(), last_tray_event: MeleeTrayEvent::Disconnected }
    }

    fn get_player_port(&mut self) -> Option<u8> { self.mem.read::<u8>(R13!(0x5108)) }
    fn get_slippi_player_port(&mut self) -> Option<u8> { self.mem.read_msrb(MSRBOffset::MsrbLocalPlayerIndex) }
    fn get_opp_name(&mut self) -> Option<String> { self.mem.read_msrb_string::<31>(MSRBOffset::MsrbOppName) }
    fn get_player_connect_code(&mut self, port: u8) -> Option<String> {
        const PLAYER_CONNECTCODE_OFFSETS: [MSRBOffset; 4] = [MSRBOffset::MsrbP1ConnectCode, MSRBOffset::MsrbP2ConnectCode, MSRBOffset::MsrbP3ConnectCode, MSRBOffset::MsrbP4ConnectCode];
        self.mem.read_msrb_string_shift_jis::<10>(PLAYER_CONNECTCODE_OFFSETS[port as usize])
    }
    fn get_character_selection(&mut self, port: u8) -> Option<MeleeCharacter> {
        // 0x04 = character, 0x05 = skin (reference: https://github.com/bkacjios/m-overlay/blob/master/source/modules/games/GALE01-2.lua#L199-L202)
        const PLAYER_SELECTION_BLOCKS: [u32; 4] = [0x8043208B, 0x80432093, 0x8043209B, 0x804320A3];
        self.mem.read::<u8>(PLAYER_SELECTION_BLOCKS[port as usize] + 0x04).and_then(|v| MeleeCharacter::try_from(v).ok())
    }
    fn timer_mode(&mut self) -> TimerMode {
        const MATCH_INIT: u32 = 0x8046DB68; // first byte, reference: https://github.com/akaneia/m-ex/blob/master/MexTK/include/match.h#L136
        self.mem.read::<u8>(MATCH_INIT).and_then(|v| {
            for timer_mode in TimerMode::iter() {
                let val = timer_mode as u8;
                if v & val == val {
                    return Some(timer_mode);
                }
            }
            None
        }).unwrap_or(TimerMode::Countup)
    }
    fn game_time(&mut self) -> i64 { self.mem.read::<u32>(0x8046B6C8).and_then(|v| Some(v)).unwrap_or(0) as i64 }
    fn matchmaking_type(&mut self) -> Option<MatchmakingMode> {
        self.mem.read_msrb::<u8>(MSRBOffset::MsrbConnectionState).and_then(|v| MatchmakingMode::try_from(v).ok())
    }
    fn slippi_online_scene(&mut self) -> Option<SlippiMenuScene> { self.mem.read::<u8>(R13!(0x5060)).and_then(|v| SlippiMenuScene::try_from(v).ok()) }
    /*fn game_variant(&mut self) -> Option<MeleeGameVariant> {
        const GAME_ID_ADDR: u32 = 0x80000000;
        const GAME_ID_LEN: usize = 0x06;

        let game_id = self.mem.read_string::<GAME_ID_LEN>(GAME_ID_ADDR);
        if game_id.is_none() {
            return None;
        }
        return match game_id.unwrap().as_str() {
            "GALE01" => Some(MeleeGameVariant::Vanilla),
            "GTME01" => Some(MeleeGameVariant::UnclePunch),
            _ => None
        }
    }*/

    
    fn get_melee_scene(&mut self) -> Option<MeleeScene> {
        const MAJOR_SCENE: u32 = 0x80479D30;
        const MINOR_SCENE: u32 = 0x80479D33;
        let scene_tuple = (self.mem.read::<u8>(MAJOR_SCENE).unwrap_or(0), self.mem.read::<u8>(MINOR_SCENE).unwrap_or(0));

        // Print the scene_tuple to the console
        println!("Major Scene: {:?}", self.mem.read::<u8>(MAJOR_SCENE).unwrap_or(0));
        println!("Minor Scene: {:?}", self.mem.read::<u8>(MAJOR_SCENE).unwrap_or(0));
        match scene_tuple {
            (0, 0) => Some(MeleeScene::MainMenu),
            (1, 0) => Some(MeleeScene::MainMenu),
            (1, 1) => Some(MeleeScene::MainMenu),
            (2, 2) => Some(MeleeScene::VsMode),
            (43, 1) => Some(MeleeScene::UnclePunch),
            (28, 2) => Some(MeleeScene::TrainingMode),
            (28, 28) => Some(MeleeScene::TrainingMode),
            (8, 2) => Some(MeleeScene::SlippiOnline(self.slippi_online_scene())),
            (8, 0) => Some(MeleeScene::SlippiCss(self.slippi_online_scene())),
            (8, 8) => Some(MeleeScene::SlippiCss(self.slippi_online_scene())),
            (32, 1) => Some(MeleeScene::HomeRunContest),
            (15, 1) => Some(MeleeScene::TargetTest(self.get_stage())),
            (33, 1) => Some(MeleeScene::MultiManMelee(MultiManVariant::TenMan)),
            (34, 1) => Some(MeleeScene::MultiManMelee(MultiManVariant::HundredMan)),
            (35, 1) => Some(MeleeScene::MultiManMelee(MultiManVariant::ThreeMinute)),
            (36, 1) => Some(MeleeScene::MultiManMelee(MultiManVariant::FifteenMinute)),
            (37, 1) => Some(MeleeScene::MultiManMelee(MultiManVariant::Endless)),
            (38, 1) => Some(MeleeScene::MultiManMelee(MultiManVariant::Cruel)),
            _ => None
        }
    }
    fn get_stage(&mut self) -> Option<MeleeStage> {
        self.mem.read::<u8>(0x8049E6C8 + 0x88 + 0x03).and_then(|v| MeleeStage::try_from(v).ok())
    }
    fn get_character(&mut self, player_id: u8) -> Option<MeleeCharacter> {
        const PLAYER_BLOCKS: [u32; 4] = [0x80453080, 0x80453F10, 0x80454DA0, 0x80455C30];
        self.mem.read::<u8>(PLAYER_BLOCKS[player_id as usize] + 0x07).and_then(|v| MeleeCharacter::try_from(v).ok())
    }

    pub fn run(&mut self, stop_signal: CancellationToken, discord_send: tokio::sync::mpsc::Sender<DiscordClientRequest>, tray_send: std::sync::mpsc::Sender<MeleeTrayEvent>) {
        const RUN_INTERVAL: u64 = 1000;
        macro_rules! send_discord_msg {
            ($req:expr) => {
                if self.last_payload != $req {
                    let _ = discord_send.blocking_send($req);
                    self.last_payload = $req;
                }
            };
        }

        loop {
            if stop_signal.is_cancelled() {
                return;
            }
            if !self.mem.has_process() {
                println!("{}", if self.mem.find_process() { "Found" } else { "Searching process..." });
            } else {
                self.mem.check_process_running();
            }

            {
                let has_process = self.mem.has_process();
                if has_process == (self.last_tray_event == MeleeTrayEvent::Disconnected) {
                    let tray_ev = if has_process { MeleeTrayEvent::Connected } else { MeleeTrayEvent::Disconnected };
                    self.last_tray_event = tray_ev;
                    let _ = tray_send.send(tray_ev);
                }
            }

            CONFIG.with_ref(|c| {
                // self.get_game_variant();
                let gamemode_opt: Option<MeleeScene> = self.get_melee_scene();

                if gamemode_opt.is_some() {
                    let gamemode: MeleeScene = gamemode_opt.unwrap();

                    // Check if we are queueing a game
                    if c.slippi.enabled && c.slippi.show_queueing && match gamemode {
                        MeleeScene::SlippiCss(scene) =>
                            scene.and_then(|s| Some(s.is_enabled(c))).unwrap_or(true),
                        _ => false
                    } {
                        match self.matchmaking_type() {
                            Some(MatchmakingMode::Initializing) | Some(MatchmakingMode::Matchmaking) => {
                                let port_op = self.get_player_port();
                                if !port_op.is_none() {
                                    let port = port_op.unwrap();
                                    let character = if c.global.show_in_game_character { self.get_character_selection(port) } else { Some(MeleeCharacter::Hidden) };
                                    match gamemode {
                                        MeleeScene::SlippiCss(scene) => {
                                            let request = DiscordClientRequest::queue(
                                                scene,
                                                character
                                            );
                                            send_discord_msg!(request.clone());
                                        },
                                        _ => {/* shouldn't happen */}
                                    }
                                }
                            }
                            Some(MatchmakingMode::Idle) => {
                                let port_op = self.get_player_port();
                                if !port_op.is_none() {
                                    let port = port_op.unwrap();
                                    let character = if c.global.show_in_game_character { self.get_character_selection(port) } else { Some(MeleeCharacter::Hidden) };
                                    match gamemode {
                                        MeleeScene::SlippiCss(scene) => {
                                            let request = DiscordClientRequest::idle(
                                                scene,
                                                character
                                            );
                                            send_discord_msg!(request.clone());
                                        },
                                        _ => {/* shouldn't happen */}
                                    }
                                }
                            }
                            Some(_) => {
                                send_discord_msg!(DiscordClientRequest::clear());
                            }, // sometimes it's none, probably because the pointer indirection changes during the asynchronous memory requests
                            _ => {}
                        }
                    // Else, we want to see if the current game mode is enabled in the config (we're in-game)
                    } else if match gamemode {
                        
                        MeleeScene::MainMenu => true,
                        MeleeScene::SlippiCss(_) => false, // if we are in css, ignore
                        MeleeScene::SlippiOnline(scene) => c.slippi.enabled &&
                            scene.and_then(|s| Some(s.is_enabled(c))).unwrap_or(true),
                        MeleeScene::UnclePunch => c.uncle_punch.enabled,
                        MeleeScene::TrainingMode => c.training_mode.enabled,
                        MeleeScene::VsMode => c.vs_mode.enabled,
                        MeleeScene::HomeRunContest => c.stadium.enabled && c.stadium.hrc.enabled,
                        MeleeScene::TargetTest(_) => c.stadium.enabled && c.stadium.btt.enabled,
                        MeleeScene::MultiManMelee(_) => c.stadium.enabled && c.stadium.mmm.enabled
                    } {
                        let game_time = self.game_time();
                        let timestamp = if c.global.show_in_game_time {
                            DiscordClientRequestTimestamp {
                                mode: match self.timer_mode() {
                                    TimerMode::Countdown => DiscordClientRequestTimestampMode::End,
                                    TimerMode::Frozen => DiscordClientRequestTimestampMode::Static,
                                    _ => DiscordClientRequestTimestampMode::Start
                                },
                                timestamp: if self.timer_mode() == TimerMode::Countdown { current_unix_time() + game_time } else { current_unix_time() - game_time }
                            }
                        } else {
                            DiscordClientRequestTimestamp::none()
                        };
                        let player_index = match gamemode {
                            MeleeScene::VsMode => self.get_player_port(),
                            MeleeScene::SlippiOnline(_) => self.get_slippi_player_port(),
                            _ => Some(0u8) // default to port 1, mostly the case in single player modes like training mode/unclepunch
                        }.unwrap_or(0u8);
                        
                        let request = if let MeleeScene::MainMenu = gamemode {
                            // For main menu, do not show character or stage
                            DiscordClientRequest::main_menu()
                        } else {
                            // For other game modes, construct the request normally
                            DiscordClientRequest::game(
                                match gamemode { MeleeScene::TargetTest(scene) => scene, _ => self.get_stage() },
                                if c.global.show_in_game_character { self.get_character(player_index) } else { Some(MeleeCharacter::Hidden) },
                                gamemode,
                                timestamp,
                                if match gamemode { MeleeScene::SlippiOnline(_) => true, _ => false } && c.slippi.show_opponent_name { self.get_opp_name() } else { None }
                            )
                        };
                    
                        send_discord_msg!(request.clone());
                    } else {
                        send_discord_msg!(DiscordClientRequest::clear());
                    }
                } else if self.last_payload.req_type != DiscordClientRequestType::Clear {
                    send_discord_msg!(DiscordClientRequest::clear());
                }
            });

            sleep(RUN_INTERVAL);
        }
    }
}