use std::mem;

use super::dolphin_mem::DolphinMemory;

const MATCH_STRUCT_LEN: isize = 0x138;

// reference: https://github.com/project-slippi/slippi-ssbm-asm/blob/0be644aff85986eae17e96f4c98b3342ab087d05/Online/Online.s#L311-L344
#[derive(Clone, Copy)]
pub enum MSRBOffset {
    MsrbConnectionState = 0, // u8, matchmaking state defined above
    MsrbIsLocalPlayerReady = Self::MsrbConnectionState as isize + 1, // bool
    MsrbIsRemotePlayerReady = Self::MsrbIsLocalPlayerReady as isize + 1, // bool
    MsrbLocalPlayerIndex = Self::MsrbIsRemotePlayerReady as isize + 1, // u8
    MsrbRemotePlayerIndex = Self::MsrbLocalPlayerIndex as isize + 1, // u8s
    MsrbRngOffset = Self::MsrbRemotePlayerIndex as isize + 1, // u32
    MsrbDelayFrames = Self::MsrbRngOffset as isize + 4, // u8
    MsrbUserChatmsgId = Self::MsrbDelayFrames as isize + 1, // u8
    MsrbOppChatmsgId = Self::MsrbUserChatmsgId as isize + 1, // u8
    MsrbChatmsgPlayerIndex = Self::MsrbOppChatmsgId as isize + 1, // u8
    MsrbVsLeftPlayers = Self::MsrbChatmsgPlayerIndex as isize + 1, // u32 player ports 0xP1P2P3PN
    MsrbVsRightPlayers = Self::MsrbVsLeftPlayers as isize + 4, // u32 player ports 0xP1P2P3PN
    MsrbLocalName = Self::MsrbVsRightPlayers as isize + 4, // char[31]
    MsrbP1Name = Self::MsrbLocalName as isize + 31, // char[31]
    MsrbP2Name = Self::MsrbP1Name as isize + 31, // char[31]
    MsrbP3Name = Self::MsrbP2Name as isize + 31, // char[31]
    MsrbP4Name = Self::MsrbP3Name as isize + 31, // char[31]
    MsrbOppName = Self::MsrbP4Name as isize + 31, // char[31]
    MsrbP1ConnectCode = Self::MsrbOppName as isize + 31, // char[10] hashtag is shift-jis
    MsrbP2ConnectCode = Self::MsrbP1ConnectCode as isize + 10, // char[10] hashtag is shift-jis
    MsrbP3ConnectCode = Self::MsrbP2ConnectCode as isize + 10, // char[10] hashtag is shift-jis
    MsrbP4ConnectCode = Self::MsrbP3ConnectCode as isize + 10, // char[10] hashtag is shift-jis
    MsrbP1SlippiUid = Self::MsrbP4ConnectCode as isize + 10, // char[29]
    MsrbP2SlippiUid = Self::MsrbP1SlippiUid as isize + 29, // char[29]
    MsrbP3SlippiUid = Self::MsrbP2SlippiUid as isize + 29, // char[29]
    MsrbP4SlippiUid = Self::MsrbP3SlippiUid as isize + 29, // char[29]
    MsrbErrorMsg = Self::MsrbP4SlippiUid as isize + 29, // char[241]
    ErrorMessageLen = 241,
    MsrbGameInfoBlock = Self::MsrbErrorMsg as isize + Self::ErrorMessageLen as isize, // MATCH_STRUCT_LEN
    MsrbMatchId = Self::MsrbGameInfoBlock as isize + MATCH_STRUCT_LEN,   // char[51]
    MsrbSize = Self::MsrbMatchId as isize + 51,
}

impl DolphinMemory {
    fn msrb_ptr(&mut self) -> Option<u32> {
        const CSSDT_BUF_ADDR: u32 = 0x80005614; // reference: https://github.com/project-slippi/slippi-ssbm-asm/blob/0be644aff85986eae17e96f4c98b3342ab087d05/Online/Online.s#L31
        self.pointer_indirection(CSSDT_BUF_ADDR, 2)
    }
    pub fn read_msrb<T: Sized>(&mut self, offset: MSRBOffset) -> Option<T> where [u8; mem::size_of::<T>()]: {
        self.msrb_ptr().and_then(|ptr| self.read::<T>(ptr + offset as u32))
    }

    pub fn read_msrb_string<const LEN: usize>(&mut self, offset: MSRBOffset) -> Option<String> where [u8; mem::size_of::<[u8; LEN]>()]: {
        self.msrb_ptr().and_then(|ptr| self.read_string::<LEN>(ptr + offset as u32))
    }

    pub fn read_msrb_string_shift_jis<const LEN: usize>(&mut self, offset: MSRBOffset) -> Option<String> where [u8; mem::size_of::<[u8; LEN]>()]: {
        self.msrb_ptr().and_then(|ptr| self.read_string_shift_jis::<LEN>(ptr + offset as u32))
    }
}
