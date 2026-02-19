//! Implements potential desync ISO detection. The function(s) in this module should typically
//! be called from a background thread due to processing time.

use std::fs::File;
use std::sync::{Arc, Mutex};

use chksum::chksum;
use chksum::hash::MD5;

use dolphin_integrations::{Color, Dolphin, Duration, Log};

/// Result of an ISO MD5 check after hashing completes.
#[derive(Clone, Debug)]
pub enum IsoMd5CheckResult {
    /// Hashing finished and this ISO is not on the known desync list.
    SafeIso { hash: String },

    /// Hashing finished and this ISO is on the known desync list.
    KnownDesyncIso { hash: String },

    /// Hashing failed before a valid hash could be produced.
    Failed,
}

/// Current lifecycle status of the ISO MD5 check.
#[derive(Clone, Debug)]
pub enum IsoMd5CheckState {
    NotStarted,
    InProgress,
    Complete(IsoMd5CheckResult),
}

impl Default for IsoMd5CheckState {
    fn default() -> Self {
        Self::NotStarted
    }
}

impl IsoMd5CheckState {
    pub(crate) fn iso_hash(&self) -> Option<&str> {
        match self {
            IsoMd5CheckState::Complete(IsoMd5CheckResult::SafeIso { hash })
            | IsoMd5CheckState::Complete(IsoMd5CheckResult::KnownDesyncIso { hash }) => Some(hash.as_str()),
            _ => None,
        }
    }
}

/// ISO hashes that are known to cause problems. We alert the player
/// if we detect that they're running one.
const KNOWN_DESYNC_ISOS: [&str; 10] = [
    "23d6baef06bd65989585096915da20f2",
    "27a5668769a54cd3515af47b8d9982f3",
    "5805fa9f1407aedc8804d0472346fc5f",
    "9bb3e275e77bb1a160276f2330f93931",
    "8f4d23152be3138b40e7e75a8423da23", // Diet Melee 64 v1.0.2
    "80d765b45265c2d09b3b6dc211eb3364", // Diet Melee Classic v1.0.2
    "da02952aeb9c3b62c4375a3578b7ff61", // Diet Melee Crystal v1.0.2
    "2bf0de184f82313c5e8bb2681a17600a", // Diet Melee Crystal v1.0.3
    "1f01ed5d8dda6e3eb0402fc6e8f8f36a", // Diet Melee Classic v1.0.3
    "d19d367683fd9f94453bf4e588d26d7d", // Diet Melee 64 v1.0.3
];

/// Computes an MD5 hash of the ISO at `iso_path` and writes the result to
/// `iso_md5_check_state`.
///
/// This function is currently more defensive than it probably needs to be, but while
/// we move things into Rust I'd like to reduce the chances of anything panic'ing back
/// into C++ since that can produce undefined behavior. This just handles every possible
/// failure gracefully - however seemingly rare - and simply logs the error.
pub fn run(iso_md5_check_state: Arc<Mutex<IsoMd5CheckState>>, iso_path: String) {
    set_iso_md5_check_state(&iso_md5_check_state, IsoMd5CheckState::InProgress);

    let digest = match File::open(&iso_path) {
        Ok(file) => match chksum::<MD5, _>(file) {
            Ok(digest) => digest,

            Err(error) => {
                tracing::error!(target: Log::SlippiOnline, ?error, "Unable to produce ISO MD5 Hash");
                set_iso_md5_check_state(&iso_md5_check_state, IsoMd5CheckState::Complete(IsoMd5CheckResult::Failed));

                return;
            },
        },

        Err(error) => {
            tracing::error!(target: Log::SlippiOnline, ?error, "Unable to open ISO for MD5 hashing");
            set_iso_md5_check_state(&iso_md5_check_state, IsoMd5CheckState::Complete(IsoMd5CheckResult::Failed));

            return;
        },
    };

    let hash = format!("{:x}", digest);

    if !KNOWN_DESYNC_ISOS.contains(&hash.as_str()) {
        tracing::info!(target: Log::SlippiOnline, iso_md5_hash = ?hash);

        set_iso_md5_check_state(
            &iso_md5_check_state,
            IsoMd5CheckState::Complete(IsoMd5CheckResult::SafeIso { hash }),
        );

        return;
    }

    // Dump it into the logs as well in case we're ever looking at a user's
    // logs - may end up being faster than trying to debug with them.
    tracing::warn!(
        target: Log::SlippiOnline,
        iso_md5_hash = ?hash,
        "Potential desync ISO detected"
    );

    // This has more line breaks in the C++ version and I frankly do not have the context as to
    // why they were there - some weird string parsing issue...?
    //
    // Settle on 2 (4 before) as a middle ground I guess.
    Dolphin::add_osd_message(
        Color::Red,
        Duration::Custom(20000),
        "\n\nCAUTION: You are using an ISO that is known to cause desyncs",
    );

    set_iso_md5_check_state(
        &iso_md5_check_state,
        IsoMd5CheckState::Complete(IsoMd5CheckResult::KnownDesyncIso { hash }),
    );
}

fn set_iso_md5_check_state(iso_md5_check_state: &Mutex<IsoMd5CheckState>, new_state: IsoMd5CheckState) {
    match iso_md5_check_state.lock() {
        Ok(mut iso_md5_check_state) => {
            *iso_md5_check_state = new_state;
        },

        Err(error) => {
            tracing::error!(target: Log::SlippiOnline, ?error, "Unable to lock iso_md5_check_state");
        },
    }
}
