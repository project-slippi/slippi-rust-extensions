//! The Slippi matchmaking state, pushed from the C++ side once per online
//! frame, and the small enums it maps to. This is what drives menu and queue
//! presence. There are no memory reads.

/// `SlippiMatchmaking::OnlinePlayMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnlineMode {
    Ranked,
    Unranked,
    Direct,
    Teams,
    Party,
}

impl OnlineMode {
    fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::Ranked,
            1 => Self::Unranked,
            2 => Self::Direct,
            3 => Self::Teams,
            4 => Self::Party,
            _ => return None,
        })
    }

    /// The human-readable label for this online mode, shown in presence text.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Ranked => "Ranked",
            Self::Unranked => "Unranked",
            Self::Direct => "Direct",
            Self::Teams => "Teams",
            Self::Party => "Party",
        }
    }
}

/// `SlippiMatchmaking::ProcessState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ProcessState {
    #[default]
    Idle,
    Initializing,
    Matchmaking,
    OpponentConnecting,
    ConnectionSuccess,
    Error,
}

impl ProcessState {
    fn from_id(id: u8) -> Self {
        match id {
            1 => Self::Initializing,
            2 => Self::Matchmaking,
            3 => Self::OpponentConnecting,
            4 => Self::ConnectionSuccess,
            5 => Self::Error,
            _ => Self::Idle,
        }
    }
}

/// A snapshot of Slippi's matchmaking layer, pushed from the C++ side once per
/// online frame. Equality drives edge-detection on the presence thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MatchmakingState {
    pub process: ProcessState,
    pub mode: Option<OnlineMode>,
    pub opponent_name: Option<String>,
    /// `None` when unknown; the C++ side sends a negative rank in that case.
    pub opponent_rank: Option<i8>,
}

impl MatchmakingState {
    /// Builds the typed snapshot from the raw matchmaking values pushed over the
    /// C++ FFI each online frame.
    pub(crate) fn from_ffi(process_state: u8, online_mode: u8, opponent_name: Option<String>, opponent_rank: i8) -> Self {
        Self {
            process: ProcessState::from_id(process_state),
            mode: OnlineMode::from_id(online_mode),
            opponent_name,
            opponent_rank: (opponent_rank >= 0).then_some(opponent_rank),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_state_maps_known_and_unknown() {
        let cases = [
            (0, ProcessState::Idle),
            (1, ProcessState::Initializing),
            (5, ProcessState::Error),
            (6, ProcessState::Idle),
            (255, ProcessState::Idle),
        ];

        for (id, expected) in cases {
            assert_eq!(ProcessState::from_id(id), expected);
        }
    }

    #[test]
    fn online_mode_includes_party() {
        assert_eq!(OnlineMode::from_id(4), Some(OnlineMode::Party));
        assert_eq!(OnlineMode::from_id(9), None);
    }

    #[test]
    fn from_ffi_normalizes_negative_rank() {
        let unknown = MatchmakingState::from_ffi(2, 0, None, -1);
        assert_eq!(unknown.opponent_rank, None);

        let known = MatchmakingState::from_ffi(3, 0, None, 0);
        assert_eq!(known.opponent_rank, Some(0));
    }

    #[test]
    fn from_ffi_builds_expected_state() {
        let state = MatchmakingState::from_ffi(2, 0, Some("Mango".to_string()), 13);

        assert_eq!(state.process, ProcessState::Matchmaking);
        assert_eq!(state.mode, Some(OnlineMode::Ranked));
        assert_eq!(state.opponent_name.as_deref(), Some("Mango"));
        assert_eq!(state.opponent_rank, Some(13));
    }
}
