//! The live Melee scene and character-select state, pushed from the C++ side
//! over the menu-frame heartbeat. This is what drives presence for the
//! character-select screen and the offline scenes (training, the 1P modes,
//! offline Vs) that never reach the replay stream, so unlike everything else in
//! this crate it depends on Dolphin telling us the current scene. There are no
//! memory reads here; the bytes arrive over FFI exactly like `MatchmakingState`.

/// External character ID sentinel for an empty or disabled character-select
/// port. Mirrors the `0xFF` the C++ side writes for a port no one is on.
const PORT_EMPTY: u8 = 0xFF;

/// `Scene.major` for the Versus-mode group, which owns the character- and
/// stage-select screens.
const MAJOR_VS: u8 = 0x02;

/// `Scene.major` for Training mode (shared by UnclePunch).
const MAJOR_TRAINING: u8 = 0x1C;

/// `Scene.major` for the online Versus group (ranked and unranked). The
/// character-select cursor is read from RAM here because the matchmaking FFI
/// has nothing to push before a search starts.
const MAJOR_VS_ONLINE: u8 = 0x08;

/// `Scene.minor` for a character-select screen (within a mode that has one).
const MINOR_CSS: u8 = 0x00;

/// `Scene.minor` for the Versus stage-select screen.
const MINOR_SSS: u8 = 0x01;

/// A snapshot of the Melee scene controller and the character-select cards,
/// pushed from the C++ side once per menu-frame heartbeat. Equality drives
/// edge-detection on the presence thread, exactly like `MatchmakingState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SceneState {
    /// `Scene.major`: the broad mode (Versus, Training, a 1P mode, online).
    pub major: u8,

    /// `Scene.minor`: the screen within that mode (character select, stage
    /// select, in-game).
    pub minor: u8,

    /// The external character ID hovered or selected on each port; `0xFF` for
    /// an empty port. Only meaningful on the character-select screen.
    pub char_ids: [u8; 4],

    /// The 0-based port of the local player on the character-select screen.
    pub local_port: u8,

    /// The internal stage ID of the loaded stage, or 0 when none is loaded
    /// (a menu or character-select screen).
    pub stage_id: u8,
}

impl SceneState {
    pub(crate) fn from_ffi(major: u8, minor: u8, char_ids: [u8; 4], local_port: u8, stage_id: u8) -> Self {
        Self {
            major,
            minor,
            char_ids,
            local_port,
            stage_id,
        }
    }

    /// The character-select screen of any mode that has one.
    pub(crate) fn is_css(&self) -> bool {
        self.minor == MINOR_CSS && matches!(self.major, MAJOR_VS | MAJOR_TRAINING | MAJOR_VS_ONLINE)
    }

    /// The stage-select screen of any mode that has one.
    pub(crate) fn is_sss(&self) -> bool {
        self.minor == MINOR_SSS && matches!(self.major, MAJOR_VS | MAJOR_TRAINING | MAJOR_VS_ONLINE)
    }

    /// The local player's hovered character, when a real one is under the
    /// cursor. `None` on an empty port or a port off the roster.
    pub(crate) fn local_char_id(&self) -> Option<u8> {
        let id = *self.char_ids.get(self.local_port as usize)?;
        (id != PORT_EMPTY).then_some(id)
    }

    /// The internal ID of the loaded stage, when one is loaded.
    pub(crate) fn stage(&self) -> Option<u8> {
        (self.stage_id != 0).then_some(self.stage_id)
    }

    /// A label for the current offline mode, or `None` for menus and online
    /// scenes that presence handles elsewhere. Drives the `details` line for
    /// character-select, stage-select and in-mode scenes.
    pub(crate) fn mode_label(&self) -> Option<&'static str> {
        Some(match self.major {
            MAJOR_VS => "Versus",
            MAJOR_TRAINING => "Training Mode",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css(char_ids: [u8; 4], local_port: u8) -> SceneState {
        SceneState::from_ffi(MAJOR_VS, MINOR_CSS, char_ids, local_port, 0)
    }

    #[test]
    fn css_detected_for_each_mode_with_a_character_select() {
        // Versus, Training and online Versus each have a character-select screen.
        for major in [MAJOR_VS, MAJOR_TRAINING, MAJOR_VS_ONLINE] {
            assert!(SceneState::from_ffi(major, MINOR_CSS, [PORT_EMPTY; 4], 0, 0).is_css());
        }
        assert!(!SceneState::from_ffi(MAJOR_VS, MINOR_SSS, [PORT_EMPTY; 4], 0, 0).is_css());
        assert!(!SceneState::default().is_css());
    }

    #[test]
    fn stage_is_some_only_when_loaded() {
        assert_eq!(SceneState::default().stage(), None);
        assert_eq!(
            SceneState::from_ffi(MAJOR_TRAINING, 0x02, [PORT_EMPTY; 4], 0, 0x20).stage(),
            Some(0x20)
        );
    }

    #[test]
    fn local_char_id_respects_port_and_empty_sentinel() {
        // Falco (0x14) on port 1, everything else empty.
        let scene = css([PORT_EMPTY, 0x14, PORT_EMPTY, PORT_EMPTY], 1);
        assert_eq!(scene.local_char_id(), Some(0x14));

        // Local player sits on an empty port.
        let scene = css([0x14, PORT_EMPTY, PORT_EMPTY, PORT_EMPTY], 1);
        assert_eq!(scene.local_char_id(), None);
    }

    #[test]
    fn stage_select_detected_for_vs_and_training() {
        assert!(SceneState::from_ffi(MAJOR_VS, MINOR_SSS, [PORT_EMPTY; 4], 0, 0).is_sss());
        assert!(SceneState::from_ffi(MAJOR_TRAINING, MINOR_SSS, [PORT_EMPTY; 4], 0, 0).is_sss());
        assert!(!SceneState::from_ffi(MAJOR_TRAINING, MINOR_CSS, [PORT_EMPTY; 4], 0, 0).is_sss());
    }

    #[test]
    fn mode_label_covers_offline_modes() {
        assert_eq!(
            SceneState::from_ffi(MAJOR_TRAINING, 0x02, [PORT_EMPTY; 4], 0, 0).mode_label(),
            Some("Training Mode")
        );
        assert_eq!(SceneState::default().mode_label(), None);
    }

    #[test]
    fn default_scene_is_unrecognized() {
        let scene = SceneState::default();
        assert!(!scene.is_css());
        assert!(!scene.is_sss());
        assert_eq!(scene.mode_label(), None);
    }
}
