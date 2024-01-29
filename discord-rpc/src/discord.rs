use discord_rich_presence::{activity::{self, Timestamps, Button}, DiscordIpc, DiscordIpcClient};

use crate::{util::current_unix_time, melee::{stage::{MeleeStage, OptionalMeleeStage}, character::{MeleeCharacter, OptionalMeleeCharacter}, MeleeScene, SlippiMenuScene, dolphin_user::get_connect_code}, rank, config::CONFIG};
use crate::util;

#[derive(Debug, PartialEq, Clone)]
pub enum DiscordClientRequestType {
    Clear,
    Queue,
    Game,
    Mainmenu,
    Idle,
}

#[derive(Debug, PartialEq, Clone)]
pub enum DiscordClientRequestTimestampMode {
    None,
    Start,
    Static, // like Start, but we never update even if the timestamp changes. Used for non-ingame actions. 
    End
}

#[derive(Debug, Clone)]
pub struct DiscordClientRequestTimestamp {
    pub mode: DiscordClientRequestTimestampMode,
    pub timestamp: i64
}

impl DiscordClientRequestTimestamp {
    pub fn none() -> Self { Self { mode: DiscordClientRequestTimestampMode::None, timestamp: 0 } }
}

// we ignore this field
impl PartialEq for DiscordClientRequestTimestamp {
    fn eq(&self, o: &Self) -> bool {
        // if the game was in pause for too long, resynchronize by saying that this payload is not the same as the other.
        // To respect the rate limit, we choose a relatively high amount of seconds
        self.mode == DiscordClientRequestTimestampMode::Static || self.timestamp.abs_diff(o.timestamp) < 15
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct DiscordClientRequest {
    pub req_type: DiscordClientRequestType,
    pub scene: Option<SlippiMenuScene>,
    pub stage: OptionalMeleeStage,
    pub character: OptionalMeleeCharacter,
    pub mode: String,
    pub timestamp: DiscordClientRequestTimestamp,
    pub opp_name: Option<String>
}

impl Default for DiscordClientRequest {
    fn default() -> Self {
        DiscordClientRequest {
            req_type: DiscordClientRequestType::Clear,
            scene: None,
            stage: OptionalMeleeStage(None),
            character: OptionalMeleeCharacter(None),
            mode: "".into(),
            timestamp: DiscordClientRequestTimestamp {
                mode: DiscordClientRequestTimestampMode::Static,
                timestamp: current_unix_time(),
            },
            opp_name: None
        }
    }
}

impl DiscordClientRequest {
    pub fn clear() -> Self { Default::default() }
    pub fn queue(scene: Option<SlippiMenuScene>, character: Option<MeleeCharacter>) -> Self {
        Self {
            req_type: DiscordClientRequestType::Queue,
            scene,
            character: OptionalMeleeCharacter(character),
            ..Default::default()
        }
    }
    pub fn idle(scene: Option<SlippiMenuScene>, character: Option<MeleeCharacter>) -> Self {
        Self {
            req_type: DiscordClientRequestType::Idle,
            scene,
            character: OptionalMeleeCharacter(character),
            ..Default::default()
        }
    }
    pub fn main_menu() -> Self {
        Self {
            req_type: DiscordClientRequestType::Mainmenu,
            ..Default::default()
        }
    }
    pub fn game(stage: Option<MeleeStage>, character: Option<MeleeCharacter>, mode: MeleeScene, timestamp: DiscordClientRequestTimestamp, opp_name: Option<String>) -> Self {
        Self {
            req_type: DiscordClientRequestType::Game,
            stage: OptionalMeleeStage(stage),
            character: OptionalMeleeCharacter(character),
            mode: mode.to_string(),
            timestamp,
            opp_name,
            ..Default::default()
        }
    }
}

pub struct DiscordClient {
    client: DiscordIpcClient
}

impl DiscordClient {
    pub fn clear(&mut self) {
        self.client.clear_activity().unwrap();
    }
    pub async fn queue(&mut self, scene: Option<SlippiMenuScene>, character: OptionalMeleeCharacter) {
        let mut large_image = "slippi".into();
        let mut large_text = "Searching".into();
        let mut buttons = Vec::with_capacity(1);
        let mut _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = "".into();
        if CONFIG.with_ref(|c| c.slippi.ranked.show_rank) {
            let connect_code_opt = get_connect_code();
            if connect_code_opt.is_some() {
                let connect_code = connect_code_opt.unwrap();
                if connect_code.is_valid() {
                    let fmt_code = connect_code.as_url();

                    let rank_info = rank::get_rank_info(fmt_code.as_str()).await.unwrap();
                    large_image = rank_info.name.to_lowercase().replace(" ", "_");
                    large_text = format!("{} | {} ELO", rank_info.name, util::round(rank_info.elo, 2));
                    if CONFIG.with_ref(|c| c.slippi.ranked.show_view_ranked_profile_button) {
                        _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = format!("https://slippi.gg/user/{}", fmt_code.as_str());
                        buttons.push(Button::new("Get Slippi", "https://slippi.gg/"));
                        buttons.push(Button::new("View Ranked Profile", _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it.as_str()));
                    }
                }
            }
        }

        self.client.set_activity(
            activity::Activity::new()
                .assets({
                    let mut activity = activity::Assets::new();
                    if !large_image.is_empty() { activity = activity.large_image(large_image.as_str()); }
                    if !large_text.is_empty() { activity = activity.large_text(large_text.as_str()); }
                    activity.small_image(character.as_discord_resource().as_str())
                        .small_text(character.to_string().as_str())
                })
                .buttons(buttons)
                .timestamps(self.current_timestamp())
                .details(scene.and_then(|v| Some(v.to_string())).unwrap_or("".into()).as_str())
                .state("In Queue")
        ).unwrap()
        
    }
    pub async fn main_menu(&mut self) {
        let mut large_image = "slippi".into();
        let mut large_text = "Idle".into();
        let mut buttons = Vec::with_capacity(1);
        let mut _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = "".into();
        if CONFIG.with_ref(|c| c.slippi.ranked.show_rank) {
            let connect_code_opt = get_connect_code();
            if connect_code_opt.is_some() {
                let connect_code = connect_code_opt.unwrap();
                if connect_code.is_valid() {
                    let fmt_code = connect_code.as_url();
    
                    let rank_info = rank::get_rank_info(fmt_code.as_str()).await.unwrap();
                    large_image = "slippi";
                    large_text = format!("{} | {} ELO", rank_info.name, util::round(rank_info.elo, 2));
                    if CONFIG.with_ref(|c| c.slippi.ranked.show_view_ranked_profile_button) {
                        _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = format!("https://slippi.gg/user/{}", fmt_code.as_str());
                        buttons.push(Button::new("Get Slippi", "https://slippi.gg/"));
                        buttons.push(Button::new("View Ranked Profile", _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it.as_str()));
                        
                    }
                }
            }
        }
    
        self.client.set_activity(
            activity::Activity::new()
                .assets({
                    let mut activity = activity::Assets::new();
                    if !large_image.is_empty() { activity = activity.large_image(large_image); }
                    if !large_text.is_empty() { activity = activity.large_text(large_text.as_str()); }
                    activity
                })
                .buttons(buttons)
                .timestamps(self.current_timestamp())
                .details("Super Smash Bros. Melee")
                .state("Main Menu")
        ).unwrap()
    }
    
    pub async fn idle(&mut self, scene: Option<SlippiMenuScene>, character: OptionalMeleeCharacter) {
        let mut large_image = "slippi".into();
        let mut large_text = "Idle".into();
        let mut buttons = Vec::with_capacity(1);
        let mut _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = "".into();
        if CONFIG.with_ref(|c| c.slippi.ranked.show_rank) {
            let connect_code_opt = get_connect_code();
            if connect_code_opt.is_some() {
                let connect_code = connect_code_opt.unwrap();
                if connect_code.is_valid() {
                    let fmt_code = connect_code.as_url();

                    let rank_info = rank::get_rank_info(fmt_code.as_str()).await.unwrap();
                    large_image = rank_info.name.to_lowercase().replace(" ", "_");
                    large_text = format!("{} | {} ELO", rank_info.name, util::round(rank_info.elo, 2));
                    if CONFIG.with_ref(|c| c.slippi.ranked.show_view_ranked_profile_button) {
                        _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = format!("https://slippi.gg/user/{}", fmt_code.as_str());
                        buttons.push(Button::new("Get Slippi", "https://slippi.gg/"));
                        buttons.push(Button::new("View Ranked Profile", _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it.as_str()));
                    }
                }
            }
        }

        self.client.set_activity(
            activity::Activity::new()
                .assets({
                    let mut activity = activity::Assets::new();
                    if !large_image.is_empty() { activity = activity.large_image(large_image.as_str()); }
                    if !large_text.is_empty() { activity = activity.large_text(large_text.as_str()); }
                    activity.small_image(character.as_discord_resource().as_str())
                        .small_text(character.to_string().as_str())
                })
                .buttons(buttons)
                .timestamps(self.current_timestamp())
                .details(scene.and_then(|v| Some(v.to_string())).unwrap_or("".into()).as_str())
                .state("Character Selection Screen")
        ).unwrap()
        
    }
    
    pub async fn game(&mut self, stage: OptionalMeleeStage, character: OptionalMeleeCharacter, mode: String, timestamp: DiscordClientRequestTimestamp, opp_name: Option<String>) {
        let mut large_image = "slippi".into();
        let mut large_text = "Idle".into();
        let mut buttons = Vec::with_capacity(1);
        let mut _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = "".into();
        if CONFIG.with_ref(|c| c.slippi.ranked.show_rank) {
            let connect_code_opt = get_connect_code();
            if connect_code_opt.is_some() {
                let connect_code = connect_code_opt.unwrap();
                if connect_code.is_valid() {
                    let fmt_code = connect_code.as_url();

                    let rank_info = rank::get_rank_info(fmt_code.as_str()).await.unwrap();
                    large_image = rank_info.name.to_lowercase().replace(" ", "_");
                    large_text = format!("{} | {} ELO", rank_info.name, util::round(rank_info.elo, 2));
                    if CONFIG.with_ref(|c| c.slippi.ranked.show_view_ranked_profile_button) {
                        _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it = format!("https://slippi.gg/user/{}", fmt_code.as_str());
                        buttons.push(Button::new("Get Slippi", "https://slippi.gg/"));
                        buttons.push(Button::new("View Ranked Profile", _i_unfortunately_have_to_use_this_variable_because_of_rust_but_im_thankful_for_it.as_str()));
                    }
                }
            }
        }
        self.client.set_activity(
            activity::Activity::new()
                .assets(
                    activity::Assets::new()
                        .large_image(stage.as_discord_resource().as_str())
                        .large_text(stage.to_string().as_str())
                        .small_image(character.as_discord_resource().as_str())
                        .small_text(character.to_string().as_str())
                )
                .timestamps(
                    if timestamp.mode == DiscordClientRequestTimestampMode::None { Timestamps::new() }
                    else if (timestamp.mode as u8) < (DiscordClientRequestTimestampMode::End as u8) { Timestamps::new().start(timestamp.timestamp) }
                    else { Timestamps::new().end(timestamp.timestamp) })
                .buttons(buttons)
                .details(mode.as_str())
                .state(opp_name.and_then(|n| Some(format!("Playing against {}", n))).unwrap_or("In Game".into()).as_str())
        ).unwrap()
        
    }
    
    pub fn close(&mut self) {
        self.client.close().unwrap();
    }

    fn current_timestamp(&self) -> Timestamps {
        Timestamps::new().start(util::current_unix_time())
    }
}

pub fn start_client() -> Result<DiscordClient, Box<dyn std::error::Error>> {
    let mut client = DiscordIpcClient::new("1096595344600604772")?;
    client.connect()?;

    Ok(DiscordClient { client })
}