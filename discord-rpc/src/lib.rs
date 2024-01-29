// TODO Sessions - each scene has a minor 0 which is the css. if you leave the major scene, the session ends, otherwise when not in-game we show when the session started
// ^ option name "Show overall game session when not in-game" 
// TODO HRC & BTT Records in discord
// TODO Ranked match score, button "Viw opponent ranked profile", show details in stage striking already (in discord rich presence, signalize that you are in stage striking as well)
// TODO clean up melee.rs, move structs/enums away in coherent bundles

//#![windows_subsystem = "windows"]
#![feature(generic_const_exprs)]

#[macro_use]
extern crate serde_derive;

use discord::{DiscordClientRequest, DiscordClientRequestType};
use single_instance::SingleInstance;
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;
use util::sleep;

use crate::tray::MeleeTrayEvent;

mod config;
mod discord;
mod tray;
mod rank;
mod util;
mod melee;

#[tokio::main]
async fn main() {
    let instance = SingleInstance::new("SLIPPI_DISCORD_RICH_PRESENCE_MTX").unwrap();
    assert!(instance.is_single());
    let (tx, mut rx) = mpsc::channel::<DiscordClientRequest>(32);
    let (mtx, mrx) = std::sync::mpsc::channel::<MeleeTrayEvent>();

    let cancel_token = CancellationToken::new();
    let melee_cancel_token = cancel_token.child_token();
    tokio::spawn(async move {
        loop {
            let discord_tx = tx.clone();
            let tray_tx = mtx.clone();
            let c_token = melee_cancel_token.clone();
            let res = tokio::task::spawn_blocking(move || {
                let mut client = melee::MeleeClient::new();
                client.run(c_token, discord_tx, tray_tx);
            }).await;
            match res {
                Ok(_) => { /* handle successfull exit */ },
                Err(err) if err.is_panic() => {
                    // panic
                    let _ = tx.send(DiscordClientRequest::clear()).await;
                    println!("[ERROR] Melee Client crashed. Restarting...");
                    sleep(500);
                },
                Err(_) => { }
            }
        }
    });

    let discord_cancel_token = cancel_token.clone();
    tokio::spawn(async move {
        let mut discord_client = discord::start_client().unwrap();

        loop {
            if discord_cancel_token.is_cancelled() {
                break
            }
            let poll_res = rx.try_recv();
            if poll_res.is_ok() {
                let msg = poll_res.unwrap();
                println!("{:?}", msg);
                match msg.req_type {
                    DiscordClientRequestType::Queue => discord_client.queue(msg.scene, msg.character).await,
                    DiscordClientRequestType::Idle => discord_client.idle(msg.scene, msg.character).await,
                    DiscordClientRequestType::Game => discord_client.game(msg.stage, msg.character, msg.mode, msg.timestamp, msg.opp_name).await,
                    DiscordClientRequestType::Mainmenu => discord_client.main_menu().await,
                    DiscordClientRequestType::Clear => discord_client.clear()
                }
            }
            
        }
        discord_client.close();
    });

    tray::run_tray(mrx); // synchronous

    // cleanup
    cancel_token.cancel();
}