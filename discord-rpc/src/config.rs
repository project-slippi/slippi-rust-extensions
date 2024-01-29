use preferences::{AppInfo, Preferences};
use ruspiro_singleton::Singleton;

use crate::melee::SlippiMenuScene;

pub const APP_INFO: AppInfo = AppInfo {
    name: "conf",
    author: "Slippi Discord Integration",
};
const PREFS_KEY: &str = "app_config";

pub static CONFIG: Singleton<AppConfig> = Singleton::lazy(&|| {
    match AppConfig::load(&APP_INFO, PREFS_KEY) {
        Ok(cfg) => cfg,
        Err(_) => AppConfig::default()
    }
});

structstruck::strike! {
    #[strikethrough[derive(Serialize, Deserialize, PartialEq, Debug)]]
    pub struct AppConfig {
        pub global: struct {
            pub show_in_game_character: bool,
            pub show_in_game_time: bool
        },
        pub slippi: struct {
            pub enabled: bool,
            pub show_queueing: bool,
            pub show_opponent_name: bool,
            pub ranked: struct {
                pub enabled: bool,
                pub show_rank: bool,
                pub show_view_ranked_profile_button: bool,
                pub show_score: bool
            },
            pub unranked: struct {
                pub enabled: bool
            },
            pub direct: struct {
                pub enabled: bool
            },
            pub teams: struct {
                pub enabled: bool
            }
        },
        pub uncle_punch: struct {
            pub enabled: bool
        },
        pub vs_mode: struct {
            pub enabled: bool
        },
        pub training_mode: struct {
            pub enabled: bool
        },
        pub stadium: struct {
            pub enabled: bool,
            pub hrc: struct {
                pub enabled: bool
            },
            pub btt: struct {
                pub enabled: bool,
                pub show_stage_name: bool
            },
            pub mmm: struct {
                pub enabled: bool
            }
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            global: Global {
                show_in_game_character: true,
                show_in_game_time: true
            },
            slippi: Slippi {
                enabled: true,
                show_queueing: true,
                show_opponent_name: true,
                ranked: Ranked {
                    enabled: true,
                    show_rank: true,
                    show_view_ranked_profile_button: true,
                    show_score: true
                },
                unranked: Unranked { enabled: true },
                direct: Direct { enabled: true },
                teams: Teams { enabled: true }
            },
            uncle_punch: UnclePunch { enabled: true },
            vs_mode: VsMode { enabled: true },
            training_mode: TrainingMode { enabled: true },
            stadium: Stadium {
                enabled: true,
                hrc: Hrc {
                    enabled: true
                },
                btt: Btt {
                    enabled: true,
                    show_stage_name: true
                },
                mmm: Mmm {
                    enabled: true
                }
            }
        }
    }
}

pub fn write_config(val: &AppConfig) {
    let _ = val.save(&APP_INFO, PREFS_KEY);
}

// Utility implementations
impl SlippiMenuScene {
    pub fn is_enabled(&self, c: &AppConfig) -> bool {
        match *self {
            SlippiMenuScene::Ranked => c.slippi.ranked.enabled,
            SlippiMenuScene::Unranked => c.slippi.unranked.enabled,
            SlippiMenuScene::Direct => c.slippi.direct.enabled,
            SlippiMenuScene::Teams => c.slippi.teams.enabled
        }
    }
}