use crate::types::StartConditions;

#[derive(Default, Debug)]
pub struct GeckoManager;

impl GeckoManager {
    /// Build the Gecko code list bytes for the given match conditions.
    /// Returns (bytes, total_size_in_bytes).
    pub fn prepare_for_match(&self, conditions: &StartConditions) -> (Vec<u8>, usize) {
        // HINT: Generate your real Gecko patches here based on `conditions`.
        // For now, return a recognizable sentinel payload.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GECKO\0");
        bytes.push((conditions.stage_id & 0xFF) as u8);
        bytes.push(((conditions.stage_id >> 8) & 0xFF) as u8);
        let total = bytes.len();
        (bytes, total)
    }
}
