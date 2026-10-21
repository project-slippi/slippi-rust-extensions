//! Matchmaking client for Slippi Online.
//!
//! The client talks to the matchmaking service over a WebSocket, keeps the
//! ticket alive across reconnects, and hands the resulting match back as the
//! JSON the service produced so the Dolphin side can parse it the way it always
//! has. The UDP socket used for netplay stays on the C++ side; this crate only
//! encodes and decodes the STUN messages sent through it and classifies the
//! NAT from the answers.

mod client;
mod protocol;
mod stun;

pub use client::{MatchRequest, MatchmakingClient, MatchmakingState, URL_ENV_VAR};
pub use protocol::{Candidate, DebugOverrides};
pub use stun::{NatType, REQUEST_LEN, build_binding_request, classify_nat, parse_binding_response};
