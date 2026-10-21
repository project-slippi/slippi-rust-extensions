//! Runs two clients against a real matchmaking service and expects a match.
//!
//! Skipped unless `SLIPPI_MM_TEST_URL` points at a service started with
//! `DEV_SKIP_AUTH=true`, for example `ws://127.0.0.1:8099/v1/queue`.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use slippi_gg_api::APIClient;
use slippi_matchmaking::{DebugOverrides, MatchRequest, MatchmakingClient, MatchmakingState};
use slippi_user::UserManager;

fn client(name: &str, uid: &str, code: &str, url: &str) -> MatchmakingClient {
    let dir = std::env::temp_dir().join(format!("slippi-mm-test-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let api = APIClient::new("3.4.0");
    let user_manager = UserManager::new(api, dir, "3.4.0".into());
    user_manager.set(|user| {
        user.uid = uid.to_string();
        user.play_key = "dev".to_string();
        user.connect_code = code.to_string();
        user.display_name = name.to_string();
    });

    // SAFETY: test-only; set before any client thread starts.
    unsafe { std::env::set_var(slippi_matchmaking::URL_ENV_VAR, url) };
    MatchmakingClient::new(user_manager, "3.4.0".into())
}

fn wait_for(client: &MatchmakingClient, wanted: MatchmakingState, timeout: Duration) -> MatchmakingState {
    let start = Instant::now();
    loop {
        let state = client.state();
        if state == wanted || state == MatchmakingState::Failed {
            return state;
        }
        if start.elapsed() > timeout {
            return state;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn two_clients_get_matched() {
    let Ok(url) = std::env::var("SLIPPI_MM_TEST_URL") else {
        eprintln!("SLIPPI_MM_TEST_URL not set, skipping live service test");
        return;
    };

    let a = client("a", "test-a", "AAA#1", &url);
    let b = client("b", "test-b", "BBB#1", &url);

    let request = |port: u16, lat: f64, lon: f64| {
        // Both servers saw the same mapping, so this looks like a cone NAT
        let reflexive = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 5), port + 1);
        MatchRequest {
            mode: 1,
            connect_code: vec![],
            netplay_port: port,
            lan_addr: format!("192.168.1.2:{}", port),
            stun: [Some(reflexive); 2],
            debug: Some(DebugOverrides {
                latitude: Some(lat),
                longitude: Some(lon),
                continent: Some("NA".into()),
                mmr: Some(1500.0),
            }),
        }
    };

    a.start(request(41000, 40.71, -74.01));
    assert_eq!(
        wait_for(&a, MatchmakingState::Queued, Duration::from_secs(5)),
        MatchmakingState::Queued,
        "{}",
        a.error_message()
    );

    b.start(request(41010, 41.88, -87.63));
    assert_eq!(
        wait_for(&b, MatchmakingState::Matched, Duration::from_secs(10)),
        MatchmakingState::Matched,
        "{}",
        b.error_message()
    );
    assert_eq!(
        wait_for(&a, MatchmakingState::Matched, Duration::from_secs(10)),
        MatchmakingState::Matched,
        "{}",
        a.error_message()
    );

    let ja: serde_json::Value = serde_json::from_str(&a.take_result_json().expect("a has a result")).unwrap();
    let jb: serde_json::Value = serde_json::from_str(&b.take_result_json().expect("b has a result")).unwrap();

    assert_eq!(ja["matchId"], jb["matchId"]);
    assert!(ja["matchId"].as_str().unwrap().starts_with("mode.unranked-"));
    assert_ne!(ja["isHost"], jb["isHost"]);

    let players = ja["players"].as_array().unwrap();
    assert_eq!(players.len(), 2);
    let remote = players.iter().find(|p| p["isLocalPlayer"] == false).unwrap();
    assert_eq!(remote["uid"], "test-b");
    // The STUN-mapped port, not the local one, is what opponents get.
    assert_eq!(remote["ipAddress"], "203.0.113.5:41011");
    assert_eq!(remote["ipAddressLan"], "192.168.1.2:41010");

    assert!(a.take_result_json().is_none(), "results are handed out once");
}

#[test]
fn cancel_leaves_queue() {
    let Ok(url) = std::env::var("SLIPPI_MM_TEST_URL") else {
        return;
    };

    let c = client("c", "test-c", "CCC#1", &url);
    c.start(MatchRequest {
        mode: 1,
        netplay_port: 41020,
        debug: Some(DebugOverrides {
            latitude: Some(0.0),
            longitude: Some(0.0),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(
        wait_for(&c, MatchmakingState::Queued, Duration::from_secs(5)),
        MatchmakingState::Queued,
        "{}",
        c.error_message()
    );
    c.cancel();
    assert_eq!(c.state(), MatchmakingState::Idle);
}
