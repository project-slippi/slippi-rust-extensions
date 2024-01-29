use std::{mem::MaybeUninit, sync::{atomic::{AtomicBool, self}, Arc, mpsc::Receiver}};

use trayicon::{TrayIconBuilder, MenuBuilder};
use windows::Win32::UI::WindowsAndMessaging::{TranslateMessage, DispatchMessageA, PeekMessageA, PM_REMOVE};

use crate::{config::{CONFIG, AppConfig, write_config, APP_INFO}, util::get_appdata_file};

use {std::sync::mpsc};

struct ExtendedMenuBuilder(MenuBuilder<TrayEvents>);
impl ExtendedMenuBuilder {
    fn new() -> ExtendedMenuBuilder {
        ExtendedMenuBuilder(MenuBuilder::<TrayEvents>::new())
    }
    fn checkable(self, name: &str, is_checked: bool, id: TrayEvents) -> Self {
        ExtendedMenuBuilder(self.0.checkable(name, is_checked, id))
    }
    // checkable with enabled check
    fn cwec(self, name: &str, is_checked: bool, id: TrayEvents, enable: &[bool]) -> Self {
        ExtendedMenuBuilder(self.0.with(trayicon::MenuItem::Checkable {
            id,
            name: name.into(),
            disabled: enable.iter().any(|v| !v),
            is_checked,
            icon: None
        }))
    }
    fn submenu(self, name: &str, menu: MenuBuilder<TrayEvents>) -> Self {
        ExtendedMenuBuilder(self.0.submenu(name, menu))
    }
}
impl From<ExtendedMenuBuilder> for MenuBuilder<TrayEvents> {
    fn from(value: ExtendedMenuBuilder) -> Self {
        value.0
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum MeleeTrayEvent {
    Connected,
    Disconnected
}

#[derive(Clone, Eq, PartialEq, Debug)]
enum TrayEvents {
    _Unused,

    // Global
    ShowInGameCharacter,
    ShowInGameTime,

    // Slippi
    EnableSlippi,
    SlippiShowQueueing,
    SlippiShowOpponentName,

    SlippiEnableRanked,
    SlippiRankedShowRank,
    SlippiRankedShowViewRankedProfileButton,
    SlippiRankedShowScore,

    SlippiEnableUnranked,

    SlippiEnableDirect,

    SlippiEnableTeams,

    // Unclepunch
    EnableUnclePunch,

    // Training Mode
    EnableTrainingMode,

    // Vs. Mode
    EnableVsMode,

    // Stadium
    EnableStadium,

    StadiumEnableHRC,
    
    StadiumEnableBTT,
    StadiumBTTShowStageName,

    StadiumEnableMMM,

    // Miscallaneous
    OpenConfig,
    Quit,
}

fn build_menu(melee_connected: &Arc<AtomicBool>) -> MenuBuilder<TrayEvents> {
    CONFIG.with_ref(|c| {
        MenuBuilder::new()
        .with(trayicon::MenuItem::Item {
            id: TrayEvents::_Unused,
            name: if melee_connected.load(atomic::Ordering::Relaxed) { "✔️ Connected to Dolphin process" } else { "❌ Searching for Dolphin process..." }.into(),
            disabled: true,
            icon: None
        })
        .separator()
        .submenu(
            "Global",
                MenuBuilder::new()
                    .checkable("Show Character", c.global.show_in_game_character, TrayEvents::ShowInGameCharacter)
                    .checkable("Show In-Game Time", c.global.show_in_game_time, TrayEvents::ShowInGameTime)
        )
        .submenu(
            "Slippi Online",
            ExtendedMenuBuilder::new()
                    .checkable("Enabled", c.slippi.enabled, TrayEvents::EnableSlippi)
                    .cwec("Show activity when searching", c.slippi.show_queueing, TrayEvents::SlippiShowQueueing, &[c.slippi.enabled])
                    .cwec("Show opponent name", c.slippi.show_opponent_name, TrayEvents::SlippiShowOpponentName, &[c.slippi.enabled])
                    .submenu(
                        "Ranked",
                    ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.slippi.ranked.enabled, TrayEvents::SlippiEnableRanked, &[c.slippi.enabled])
                            .cwec("Show rank", c.slippi.ranked.show_rank, TrayEvents::SlippiRankedShowRank, &[c.slippi.enabled, c.slippi.ranked.enabled])
                            .cwec("Show \"View Ranked Profile\" button", c.slippi.ranked.show_view_ranked_profile_button, TrayEvents::SlippiRankedShowViewRankedProfileButton, &[c.slippi.enabled, c.slippi.ranked.enabled])
                            .cwec("Show match score", c.slippi.ranked.show_score, TrayEvents::SlippiRankedShowScore, &[c.slippi.enabled, c.slippi.ranked.enabled])
                            .into()
                    )
                    .submenu(
                        "Unranked",
                        ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.slippi.unranked.enabled, TrayEvents::SlippiEnableUnranked, &[c.slippi.enabled])
                            .into()
                    )
                    .submenu(
                        "Direct",
                        ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.slippi.direct.enabled, TrayEvents::SlippiEnableDirect, &[c.slippi.enabled])
                            .into()
                    )
                    .submenu(
                        "Teams",
                        ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.slippi.teams.enabled, TrayEvents::SlippiEnableTeams, &[c.slippi.enabled])
                            .into()
                    )
                    .into()
        )
        .submenu(
            "UnclePunch",
            MenuBuilder::new()
                    .checkable("Enabled", c.uncle_punch.enabled, TrayEvents::EnableUnclePunch)
        )
        .submenu(
            "Training Mode",
            MenuBuilder::new()
                    .checkable("Enabled", c.training_mode.enabled, TrayEvents::EnableTrainingMode)
        )
        .submenu(
            "Vs. Mode",
            MenuBuilder::new()
                    .checkable("Enabled", c.vs_mode.enabled, TrayEvents::EnableVsMode)
        )
        .submenu(
            "Stadium",
            ExtendedMenuBuilder::new()
                    .checkable("Enabled", c.stadium.enabled, TrayEvents::EnableStadium)
                    .submenu(
                        "Home-Run Contest",
                    ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.stadium.hrc.enabled, TrayEvents::StadiumEnableHRC, &[c.stadium.enabled])
                            .into()
                    )
                    .submenu(
                        "Target Test",
                        ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.stadium.btt.enabled, TrayEvents::StadiumEnableBTT, &[c.stadium.enabled])
                            .cwec("Show stage name", c.stadium.btt.show_stage_name, TrayEvents::StadiumBTTShowStageName, &[c.stadium.enabled])
                            .into()
                    )
                    .submenu(
                        "Multi-Man Melee",
                        ExtendedMenuBuilder::new()
                            .cwec("Enabled", c.stadium.mmm.enabled, TrayEvents::StadiumEnableMMM, &[c.stadium.enabled])
                            .into()
                    )
                    .into()
        )
        .separator()
        .item("Open Configuration File", TrayEvents::OpenConfig)
        .item("Quit", TrayEvents::Quit)
    })
}

pub fn run_tray(mrx: Receiver<MeleeTrayEvent>) {
    let melee_connected = Arc::new(AtomicBool::new(false));

    let (s, r) = mpsc::channel::<TrayEvents>();
    let icon_raw = include_bytes!("../assets/icon.ico");

    let mut tray_icon = TrayIconBuilder::new()
        .sender(s)
        .icon_from_buffer(icon_raw)
        .tooltip("Slippi Discord Integration")
        .menu(
            build_menu(&melee_connected)
        )
        .build()
        .unwrap();

    let should_end = Arc::new(AtomicBool::new(false));
    let shared_should_end = should_end.clone();
    std::thread::spawn(move || {
        let mut update_menu = || {
            tray_icon.set_menu(&build_menu(&melee_connected)).unwrap();
        };
        let mut toggle_handler = |modifier: fn(&mut AppConfig)| {
            CONFIG.with_mut(|c| { modifier(c); write_config(c); });
            update_menu();
        };

        loop {
            if let Ok(melee_ev) = mrx.try_recv() {
                melee_connected.store(melee_ev == MeleeTrayEvent::Connected, atomic::Ordering::Relaxed);
                toggle_handler(|_|{});
            }
            if let Ok(tray_ev) = r.try_recv() {
                match tray_ev {
                    TrayEvents::ShowInGameCharacter => toggle_handler(|f| f.global.show_in_game_character = !f.global.show_in_game_character),
                    TrayEvents::ShowInGameTime => toggle_handler(|f| f.global.show_in_game_time = !f.global.show_in_game_time),
        
                    TrayEvents::EnableSlippi => toggle_handler(|f| f.slippi.enabled = !f.slippi.enabled),
                    TrayEvents::SlippiShowQueueing => toggle_handler(|f| f.slippi.show_queueing = !f.slippi.show_queueing),
                    TrayEvents::SlippiShowOpponentName => toggle_handler(|f| f.slippi.show_opponent_name = !f.slippi.show_opponent_name),
        
                    TrayEvents::SlippiEnableRanked => toggle_handler(|f| f.slippi.ranked.enabled = !f.slippi.ranked.enabled),
                    TrayEvents::SlippiRankedShowRank => toggle_handler(|f| f.slippi.ranked.show_rank = !f.slippi.ranked.show_rank),
                    TrayEvents::SlippiRankedShowViewRankedProfileButton => toggle_handler(|f| f.slippi.ranked.show_view_ranked_profile_button = !f.slippi.ranked.show_view_ranked_profile_button),
                    TrayEvents::SlippiRankedShowScore => toggle_handler(|f| f.slippi.ranked.show_score = !f.slippi.ranked.show_score),
        
                    TrayEvents::SlippiEnableUnranked => toggle_handler(|f| f.slippi.unranked.enabled = !f.slippi.unranked.enabled),
        
                    TrayEvents::SlippiEnableDirect => toggle_handler(|f| f.slippi.direct.enabled = !f.slippi.direct.enabled),
        
                    TrayEvents::SlippiEnableTeams => toggle_handler(|f| f.slippi.teams.enabled = !f.slippi.teams.enabled),
        
                    TrayEvents::EnableUnclePunch => toggle_handler(|f| f.uncle_punch.enabled = !f.uncle_punch.enabled),
        
                    TrayEvents::EnableVsMode => toggle_handler(|f| f.vs_mode.enabled = !f.vs_mode.enabled),
        
                    TrayEvents::EnableTrainingMode => toggle_handler(|f| f.training_mode.enabled = !f.training_mode.enabled),

                    TrayEvents::EnableStadium => toggle_handler(|f| f.stadium.enabled = !f.stadium.enabled),

                    TrayEvents::StadiumEnableHRC => toggle_handler(|f| f.stadium.hrc.enabled = !f.stadium.hrc.enabled),

                    TrayEvents::StadiumEnableBTT => toggle_handler(|f| f.stadium.btt.enabled = !f.stadium.btt.enabled),
                    TrayEvents::StadiumBTTShowStageName => toggle_handler(|f| f.stadium.btt.show_stage_name = !f.stadium.btt.show_stage_name),

                    TrayEvents::StadiumEnableMMM => toggle_handler(|f| f.stadium.mmm.enabled = !f.stadium.mmm.enabled),
        
                    TrayEvents::OpenConfig => {
                        if let Some(conf_file) = get_appdata_file(format!("{}/{}/app_config.prefs.json", APP_INFO.author, APP_INFO.name).as_str()) {
                            if conf_file.is_file() && conf_file.exists() {
                                let _ = open::that(conf_file);
                            }
                        }
                    }
                    TrayEvents::Quit => {
                        should_end.store(true, atomic::Ordering::Relaxed);
                        break;
                    },
                    TrayEvents::_Unused => {}
                }
            }
        }
    });
    // Application message loop
    loop {
        if shared_should_end.load(atomic::Ordering::Relaxed) {
            break;
        }
        unsafe {
            let mut msg = MaybeUninit::uninit();
            let bret = PeekMessageA(msg.as_mut_ptr(), None, 0, 0, PM_REMOVE);
            if bret.as_bool() {
                TranslateMessage(msg.as_ptr());
                DispatchMessageA(msg.as_ptr());
            }
        }
    }
}