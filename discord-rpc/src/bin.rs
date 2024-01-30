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
use std::sync::{mpsc, Arc, Mutex};
use util::sleep;
use std::thread;
use std::sync::mpsc::TryRecvError;

use crate::tray::MeleeTrayEvent;

mod config;
mod discord;
mod tray;
mod rank;
mod util;
mod melee;

fn main() {
    let instance = SingleInstance::new("SLIPPI_DISCORD_RICH_PRESENCE_MTX").unwrap();
    assert!(instance.is_single());
    let (tx, rx) = mpsc::channel::<DiscordClientRequest>();
    let (mtx, mrx) = mpsc::channel::<MeleeTrayEvent>();

    let cancel_token = Arc::new(Mutex::new(false));

    {
        let cancel_token = cancel_token.clone();
        thread::spawn(move || {
            while !*cancel_token.lock().unwrap() {
                let discord_tx = tx.clone();
                let tray_tx = mtx.clone();
                // The loop is now managed by a simple spawning of a new thread after a crash
                match thread::spawn(move || {
                    let mut client = melee::MeleeClient::new();
                    client.run(discord_tx, tray_tx);
                }).join() {
                    Ok(_) => { /* handle successful exit */ },
                    Err(_) => {
                        // panic
                        let _ = tx.send(DiscordClientRequest::clear());
                        println!("[ERROR] Melee Client crashed. Restarting...");
                        sleep(500);
                    }
                }
            }
        });
    }

    let discord_cancel_token = cancel_token.clone();
    thread::spawn(move || {
        let mut discord_client = discord::start_client().unwrap();

        while !*discord_cancel_token.lock().unwrap() {
            let poll_res = rx.try_recv();
            match poll_res {
                Ok(msg) => {
                    println!("{:?}", msg);
                    match msg.req_type {
                        DiscordClientRequestType::Queue => discord_client.queue(msg.scene, msg.character),
                        DiscordClientRequestType::Idle => discord_client.idle(msg.scene, msg.character),
                        DiscordClientRequestType::Game => discord_client.game(msg.stage, msg.character, msg.mode, msg.timestamp, msg.opp_name),
                        DiscordClientRequestType::Mainmenu => discord_client.main_menu(),
                        DiscordClientRequestType::Clear => discord_client.clear()
                    }
                },
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }
        }
        discord_client.close();
    });

    tray::run_tray(mrx); // synchronous

    // cleanup
    *cancel_token.lock().unwrap() = true;
}