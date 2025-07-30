use crate::Message::*;
use dolphin_integrations::Log;
use fetcher::RankInfoFetcher;
use slippi_gg_api::APIClient;
use slippi_user::*;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

mod fetcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlippiRank {
    Unranked,
    Bronze1,
    Bronze2,
    Bronze3,
    Silver1,
    Silver2,
    Silver3,
    Gold1,
    Gold2,
    Gold3,
    Platinum1,
    Platinum2,
    Platinum3,
    Diamond1,
    Diamond2,
    Diamond3,
    Master1,
    Master2,
    Master3,
    Grandmaster,
    Count,
}

#[derive(Debug, Clone, Default)]
pub struct RankInfo {
    pub rank: i8,
    pub rating_ordinal: f32,
    pub global_placing: u8,
    pub regional_placing: u8,
    pub rating_update_count: u32,
    pub rating_change: f32,
    pub rank_change: i32,
}

#[derive(Debug)]
pub enum Message {
    FetchRank,
    RankFetcherDropped,
}

#[derive(Debug, Clone, Default)]
pub struct RankManagerData {
    pub current_rank: Option<RankInfo>,
    pub previous_rank: Option<RankInfo>,
}

#[derive(Debug)]
pub struct RankManager {
    tx: Sender<Message>,
    rank_data: Arc<Mutex<RankManagerData>>,
}

impl RankManager {
    pub fn new(api_client: APIClient, user_manager: UserManager) -> Self {
        tracing::info!(target: Log::SlippiOnline, "Initializing Slippi Rank Manager");
        let (tx, rx) = channel::<Message>();
        let rank_data = Arc::new(Mutex::new(RankManagerData::default()));

        let fetcher = RankInfoFetcher::new(api_client.clone(), user_manager.clone(), rank_data.clone());

        // Fetch rank on boot (this doesnt work, this is when dolphin opens there is no user)
        let connect_code = user_manager.get(|user| user.connect_code.clone());
        let _ = fetcher.fetch_user_rank(&connect_code);

        let _fetcher_thread = thread::Builder::new()
            .name("RankInfoFetcherThread".into())
            .spawn(move || {
                fetcher::run(fetcher, rx);
            })
            .expect("Failed to spawn RankInfoFetcherThread.");

        Self { tx, rank_data }
    }

    pub fn fetch_rank(&self) {
        // Send a message to the rank fetcher with the user's connect code
        let _ = self.tx.send(FetchRank);
    }

    pub fn get_rank(&self) -> Option<RankInfo> {
        self.rank_data.lock().unwrap().current_rank.clone()
    }

    pub fn clear(&mut self) {
        let mut data = self.rank_data.lock().unwrap();
        data.current_rank = None;
        data.previous_rank = None;
    }

    pub fn decide_rank(rating_ordinal: f32, global_placing: u8, regional_placing: u8, rating_update_count: u32) -> SlippiRank {
        if rating_update_count < 5 {
            return SlippiRank::Unranked;
        }
        if rating_ordinal > 0.0 && rating_ordinal <= 765.42 {
            return SlippiRank::Bronze1;
        }
        if rating_ordinal > 765.43 && rating_ordinal <= 913.71 {
            return SlippiRank::Bronze2;
        }
        if rating_ordinal > 913.72 && rating_ordinal <= 1054.86 {
            return SlippiRank::Bronze3;
        }
        if rating_ordinal > 1054.87 && rating_ordinal <= 1188.87 {
            return SlippiRank::Silver1;
        }
        if rating_ordinal > 1188.88 && rating_ordinal <= 1315.74 {
            return SlippiRank::Silver2;
        }
        if rating_ordinal > 1315.75 && rating_ordinal <= 1435.47 {
            return SlippiRank::Silver3;
        }
        if rating_ordinal > 1435.48 && rating_ordinal <= 1548.06 {
            return SlippiRank::Gold1;
        }
        if rating_ordinal > 1548.07 && rating_ordinal <= 1653.51 {
            return SlippiRank::Gold2;
        }
        if rating_ordinal > 1653.52 && rating_ordinal <= 1751.82 {
            return SlippiRank::Gold3;
        }
        if rating_ordinal > 1751.83 && rating_ordinal <= 1842.99 {
            return SlippiRank::Platinum1;
        }
        if rating_ordinal > 1843.0 && rating_ordinal <= 1927.02 {
            return SlippiRank::Platinum2;
        }
        if rating_ordinal > 1927.03 && rating_ordinal <= 2003.91 {
            return SlippiRank::Platinum3;
        }
        if rating_ordinal > 2003.92 && rating_ordinal <= 2073.66 {
            return SlippiRank::Diamond1;
        }
        if rating_ordinal > 2073.67 && rating_ordinal <= 2136.27 {
            return SlippiRank::Diamond2;
        }
        if rating_ordinal > 2136.28 && rating_ordinal <= 2191.74 {
            return SlippiRank::Diamond3;
        }
        if rating_ordinal >= 2191.75 && global_placing > 0 && regional_placing > 0 {
            return SlippiRank::Grandmaster;
        }
        if rating_ordinal > 2191.75 && rating_ordinal <= 2274.99 {
            return SlippiRank::Master1;
        }
        if rating_ordinal > 2275.0 && rating_ordinal <= 2350.0 {
            return SlippiRank::Master2;
        }
        if rating_ordinal > 2350.0 {
            return SlippiRank::Master3;
        }
        SlippiRank::Unranked
    }
}

impl Drop for RankManager {
    fn drop(&mut self) {
        tracing::info!(target: Log::SlippiOnline, "Dropping Rank Fetcher");
        if let Err(e) = self.tx.send(Message::RankFetcherDropped) {
            tracing::warn!(
                target: Log::SlippiOnline,
                "Failed to notify child thread that Rank Fetcher is dropping: {e}"
            );
        }
    }
}
