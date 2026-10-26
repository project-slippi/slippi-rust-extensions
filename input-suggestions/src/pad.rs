//! Reproduces the processing between a GameCube adapter report and the 8 bytes Slippi sends over the
//! network for a frame, plus how far apart two of those inputs are.
//!
//! The steps, in order:
//! 1. `GCAdapter::Input` turns the adapter's bytes for a port into buttons, sticks and triggers.
//! 2. The game's `PADRead` centers the sticks and subtracts the origin the controller reported, clamping
//!    so nothing wraps around. Melee polls the controller in analog mode 3, where every stick and trigger
//!    arrives at full precision, so nothing else changes on the way from Dolphin to the game.
//! 3. `TriggerSendInput.asm` snaps a stick resting within 2 units of center to zero.
//!
//! Only the first 8 bytes of the game's 12-byte pad report go over the network: buttons (big endian),
//! stick x/y, c-stick x/y, L, R.

/// The bytes of a pad report that are sent over the network.
pub const PAD_SIZE: usize = 8;
pub type Pad = [u8; PAD_SIZE];

/// The adapter's bytes for one port: status, two button bytes, stick x/y, c-stick x/y, L, R.
pub const ADAPTER_PORT_DATA_SIZE: usize = 9;
pub type AdapterPortData = [u8; ADAPTER_PORT_DATA_SIZE];

/// `START_SYNC_FRAME` in Online.s. The ASM zeroes every input polled before `START_SYNC_FRAME - delay`.
pub const START_SYNC_FRAME: i32 = 78;

/// Triggers at or below this value are treated as released by the ASM's rollback comparison.
const TRIGGER_DEADZONE: u8 = 42;

/// Sticks with both axes within this many units of center are snapped to center by the ASM.
const STICK_REST_UNITS: i8 = 2;

/// The analog mode Melee polls the controller in. Other modes pack the c-stick and triggers with less
/// precision, which this module does not reproduce.
const MELEE_ANALOG_MODE: u8 = 3;

/// The origin the controller reported to the game and the analog mode the game set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    /// Stick x/y, c-stick x/y, L, R as reported by the controller (unsigned, centered on 128).
    pub origin: [u8; 6],
    pub analog_mode: u8,
}

/// Converts the adapter's bytes for a port into the pad the game sends to the opponent, or `None` if the
/// game isn't polling in Melee's analog mode.
pub fn adapter_to_pad(port: &AdapterPortData, calibration: &Calibration) -> Option<Pad> {
    if calibration.analog_mode != MELEE_ANALOG_MODE {
        return None;
    }

    // A port with nothing plugged in reads as a centered pad
    let (button, raw) = if port[0] >> 4 == 0 {
        (0, [0x80, 0x80, 0x80, 0x80, 0, 0])
    } else {
        (
            adapter_buttons(port[1], port[2]),
            [port[3], port[4], port[5], port[6], port[7], port[8]],
        )
    };

    let origin = calibration.origin;
    let center = |value: u8| value.wrapping_sub(128) as i8;
    let mut stick_x = clamp_s8(center(raw[0]), center(origin[0]));
    let mut stick_y = clamp_s8(center(raw[1]), center(origin[1]));
    let mut c_x = clamp_s8(center(raw[2]), center(origin[2]));
    let mut c_y = clamp_s8(center(raw[3]), center(origin[3]));
    let trigger_l = clamp_u8(raw[4], origin[4]);
    let trigger_r = clamp_u8(raw[5], origin[5]);

    rest_clamp(&mut stick_x, &mut stick_y);
    rest_clamp(&mut c_x, &mut c_y);

    Some([
        (button >> 8) as u8,
        button as u8,
        stick_x as u8,
        stick_y as u8,
        c_x as u8,
        c_y as u8,
        trigger_l,
        trigger_r,
    ])
}

/// The button bits `GCAdapter::Input` sets from the adapter's two button bytes.
fn adapter_buttons(byte1: u8, byte2: u8) -> u16 {
    // Byte 1: A B X Y Left Right Down Up. Byte 2: Start Z R L.
    const BYTE1: [u16; 8] = [0x0100, 0x0200, 0x0400, 0x0800, 0x0001, 0x0002, 0x0004, 0x0008];
    const BYTE2: [u16; 4] = [0x1000, 0x0010, 0x0020, 0x0040];

    let set = |byte: u8, values: &[u16]| {
        (values.iter().enumerate())
            .filter(|&(bit, _)| byte & (1 << bit) != 0)
            .fold(0, |button, (_, value)| button | value)
    };
    set(byte1, &BYTE1) | set(byte2, &BYTE2)
}

/// Subtracts the origin from a centered stick axis, first clamping the value so the result can't wrap
/// around
fn clamp_s8(mut value: i8, origin: i8) -> i8 {
    if origin > 0 {
        let min = (-128i16 + origin as i16) as i8;
        if value < min {
            value = min;
        }
    } else if origin < 0 {
        let max = (127i16 + origin as i16) as i8;
        if value > max {
            value = max;
        }
    }
    value.wrapping_sub(origin)
}

/// Subtracts the origin from a trigger, stopping at zero
fn clamp_u8(value: u8, origin: u8) -> u8 {
    value.max(origin) - origin
}

/// Section 2 of `TriggerSendInput.asm`.
fn rest_clamp(x: &mut i8, y: &mut i8) {
    let at_rest = |v: i8| (-STICK_REST_UNITS..=STICK_REST_UNITS).contains(&v);
    if at_rest(*x) && at_rest(*y) {
        *x = 0;
        *y = 0;
    }
}

/// The digital buttons the ASM's rollback comparison looks at.
fn buttons(pad: &Pad) -> (u8, u8) {
    (pad[0] & 0x1F, pad[1] & 0x7F)
}

/// The digital buttons that `predicted` has wrong compared with `real`, as a bit mask.
pub fn wrong_buttons(predicted: &Pad, real: &Pad) -> u16 {
    let (p0, p1) = buttons(predicted);
    let (r0, r1) = buttons(real);
    u16::from_be_bytes([p0 ^ r0, p1 ^ r1])
}

/// How far `predicted`'s sticks and triggers are from `real`'s: the largest difference on either axis of
/// each stick plus the largest on either trigger, where two triggers inside the deadzone count as equal.
pub fn analog_error(predicted: &Pad, real: &Pad) -> u32 {
    let axis = |i: usize| (predicted[i] as i8 as i32 - real[i] as i8 as i32).unsigned_abs();
    let trigger = |i: usize| {
        let (a, b) = (predicted[i], real[i]);
        if a <= TRIGGER_DEADZONE && b <= TRIGGER_DEADZONE {
            0
        } else {
            a.abs_diff(b) as u32
        }
    };

    axis(2).max(axis(3)) + axis(4).max(axis(5)) + trigger(6).max(trigger(7))
}

/// Whether `next` is worth suggesting after `prev` was sent: only changes that can start a new action in
/// Melee, from SSBM Arte's list (leaving out tap jump from a dash and shield drop). That is pressing or
/// releasing A, B, X, Y, L, R, Z or Start; an analog trigger crossing 0.307 (light shield) either way; either
/// stick leaving the deadzone on either axis; the control stick reaching dash (x ±0.8), tap jump (y 0.6625)
/// or crouch (y -0.7) range; and the c-stick reaching a smash (x ±0.8, y ±0.6625). Moving back out of a range
/// isn't sent; the real input brings that.
pub fn is_meaningful_change(prev: &Pad, next: &Pad) -> bool {
    // Byte 0 is Start (0x10), Y, X, B, A. Byte 1 is L (0x40), R, Z (0x10), then the d-pad.
    if (prev[0] ^ next[0]) & 0x1F != 0 || (prev[1] ^ next[1]) & 0x70 != 0 {
        return true;
    }

    // 0.307 of a trigger (which Melee scales by 140) is where the ASM's trigger deadzone ends
    if (6..8).any(|i| (prev[i] > TRIGGER_DEADZONE) != (next[i] > TRIGGER_DEADZONE)) {
        return true;
    }

    // Ranges of an axis in Melee units of 1/80 (see `melee_sticks`)
    const X: usize = 0;
    const Y: usize = 1;
    const CX: usize = 2;
    const CY: usize = 3;
    let live: fn(f32) -> bool = |v| v.abs() >= 23.0; // 0.2875, past the deadzone
    let ranges: [(usize, fn(f32) -> bool); 12] = [
        (X, live),
        (Y, live),
        (X, |x| x >= 64.0),  // Dash right, 0.8
        (X, |x| x <= -64.0), // Dash left
        (Y, |y| y >= 53.0),  // Tap jump, 0.6625
        (Y, |y| y <= -56.0), // Crouch, -0.7
        (CX, live),
        (CY, live),
        (CX, |x| x >= 64.0), // Side smashes, ±0.8
        (CX, |x| x <= -64.0),
        (CY, |y| y >= 53.0), // Up and down smash, ±0.6625
        (CY, |y| y <= -53.0),
    ];

    let (before, after) = (melee_sticks(prev), melee_sticks(next));
    ranges
        .iter()
        .any(|&(axis, inside)| !inside(before[axis]) && inside(after[axis]))
}

/// The control stick and c-stick (x, y, x, y) as Melee sees them, in units of 1/80, with each stick pulled
/// back inside the unit circle. Melee's own rounding is left out; it only matters right at a range's edge.
fn melee_sticks(pad: &Pad) -> [f32; 4] {
    let clamp = |x: u8, y: u8| {
        let (x, y) = (x as i8 as f32, y as i8 as f32);
        let scale = (80.0 / x.hypot(y)).min(1.0);
        (x * scale, y * scale)
    };

    let (x, y) = clamp(pad[2], pad[3]);
    let (cx, cy) = clamp(pad[4], pad[5]);
    [x, y, cx, cy]
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTERED: Calibration = Calibration {
        origin: [128, 128, 128, 128, 0, 0],
        analog_mode: 3,
    };

    fn port(b1: u8, b2: u8, sticks: [u8; 4], l: u8, r: u8) -> AdapterPortData {
        [0x10, b1, b2, sticks[0], sticks[1], sticks[2], sticks[3], l, r]
    }

    #[test]
    fn neutral_and_unplugged_ports_read_as_zero() {
        assert_eq!(adapter_to_pad(&port(0, 0, [128; 4], 0, 0), &CENTERED), Some([0; 8]));
        // A port with a zero status nibble has nothing plugged in and reads centered, whatever its other bytes
        assert_eq!(
            adapter_to_pad(&[0x00, 0xFF, 0xFF, 0, 0, 0, 0, 255, 255], &CENTERED),
            Some([0; 8])
        );
    }

    #[test]
    fn buttons_map_to_pad_status_bits() {
        // A and Start set, d-pad up and L set
        let pad = adapter_to_pad(&port(0x01 | 0x80, 0x01 | 0x08, [128; 4], 0, 0), &CENTERED).unwrap();
        assert_eq!(pad[0], 0x11); // Start | A
        assert_eq!(pad[1], 0x48); // L | Up
    }

    #[test]
    fn sticks_are_centered_and_rest_clamped() {
        let pad = adapter_to_pad(&port(0, 0, [200, 56, 129, 127], 0, 0), &CENTERED).unwrap();
        assert_eq!(pad[2] as i8, 72);
        assert_eq!(pad[3] as i8, -72);
        // c-stick at (1, -1) rests at center
        assert_eq!(pad[4..6], [0, 0]);

        let pad = adapter_to_pad(&port(0, 0, [131, 128, 128, 128], 0, 0), &CENTERED).unwrap();
        assert_eq!(pad[2] as i8, 3);
    }

    #[test]
    fn origin_is_subtracted_and_clamped() {
        let calibration = Calibration {
            origin: [130, 126, 128, 128, 30, 0],
            analog_mode: 3,
        };
        let pad = adapter_to_pad(&port(0, 0, [140, 120, 128, 128], 100, 20), &calibration).unwrap();
        assert_eq!(pad[2] as i8, 10);
        assert_eq!(pad[3] as i8, -6);
        assert_eq!(pad[6], 70);
        assert_eq!(pad[7], 20);

        // Triggers below the origin clamp to zero
        let pad = adapter_to_pad(&port(0, 0, [128; 4], 20, 50), &calibration).unwrap();
        assert_eq!(pad[6], 0);
        assert_eq!(pad[7], 50);

        // ClampS8 with a positive origin limits the low end before subtracting
        let pad = adapter_to_pad(&port(0, 0, [0, 128, 128, 128], 0, 0), &calibration).unwrap();
        assert_eq!(pad[2] as i8, -128);
    }

    #[test]
    fn other_analog_modes_are_not_converted() {
        let calibration = Calibration {
            analog_mode: 0,
            ..CENTERED
        };
        assert_eq!(adapter_to_pad(&port(0, 0, [128; 4], 0, 0), &calibration), None);
    }

    #[test]
    fn analog_error_adds_the_worst_axis_of_each_input() {
        let real = [0x01, 0x00, 80, 0xF0, 0, 0, 100, 10];
        let predicted = [0x00, 0x00, 70, 0x10, 3, 0, 90, 30];
        // Stick y goes from -16 to 16 (32), c-stick x is off by 3, L by 10 and R is in the deadzone on both
        assert_eq!(analog_error(&predicted, &real), 45);
        assert_eq!(analog_error(&real, &real), 0);
    }

    #[test]
    fn wrong_buttons_ignores_bits_the_asm_does_not_compare() {
        assert_eq!(wrong_buttons(&[0xE0, 0x80, 0, 0, 0, 0, 0, 0], &[0; 8]), 0);
        assert_eq!(
            wrong_buttons(&[0x01, 0, 0, 0, 0, 0, 0, 0], &[0x02, 0, 0, 0, 0, 0, 0, 0]),
            0x0300
        );
    }

    #[test]
    fn only_action_triggers_are_worth_suggesting() {
        let sent = |prev: Pad, next: Pad| is_meaningful_change(&prev, &next);
        let stick = |x: i8, y: i8| [0, 0, x as u8, y as u8, 0, 0, 0, 0];
        let c_stick = |x: i8, y: i8| [0, 0, 0, 0, x as u8, y as u8, 0, 0];
        let trigger = |l: u8| [0, 0, 0, 0, 0, 0, l, 0];

        // Leaving the deadzone (0.2875 is 23) is sent; moving inside it, or back into it, is not
        assert!(!sent(stick(0, 0), stick(22, 0)));
        assert!(sent(stick(0, 0), stick(0, -23)));
        assert!(!sent(stick(23, 0), stick(0, 0)));

        // Dash at 0.8 (64) either way, but leaving dash range is not sent
        assert!(!sent(stick(30, 0), stick(63, 0)));
        assert!(sent(stick(30, 0), stick(66, 0)));
        assert!(sent(stick(-30, 0), stick(-64, 0)));
        assert!(!sent(stick(70, 0), stick(40, 0)));
        // At a diagonal the unit circle pulls x back: (66, 50) is about (63.8, 48.3), short of a dash
        assert!(!sent(stick(30, 50), stick(66, 50)));

        // Tap jump at 0.6625 (53), crouch at -0.7 (-56)
        assert!(sent(stick(0, 30), stick(0, 53)));
        assert!(!sent(stick(0, -30), stick(0, -55)));
        assert!(sent(stick(0, -30), stick(0, -56)));

        // C-stick smashes at ±0.8 sideways and ±0.6625 up and down
        assert!(sent(c_stick(30, 0), c_stick(-64, 0)));
        assert!(sent(c_stick(0, -30), c_stick(0, -53)));
        assert!(!sent(c_stick(0, 30), c_stick(0, 52)));

        // Light shield: a trigger crossing 0.307 (43) either way, but not moving further in
        assert!(sent(trigger(42), trigger(43)));
        assert!(sent(trigger(43), trigger(42)));
        assert!(!sent(trigger(50), trigger(120)));

        // Presses and releases of A, B, X, Y, L, R, Z and Start, but never the d-pad
        assert!(sent([0; 8], [0x01, 0, 0, 0, 0, 0, 0, 0]));
        assert!(sent([0x01, 0, 0, 0, 0, 0, 0, 0], [0; 8]));
        assert!(sent([0x02, 0, 0, 0, 0, 0, 0, 0], [0; 8]));
        assert!(sent([0; 8], [0, 0x40, 0, 0, 0, 0, 0, 0]));
        assert!(sent([0, 0x10, 0, 0, 0, 0, 0, 0], [0; 8]));
        assert!(!sent([0; 8], [0, 0x08, 0, 0, 0, 0, 0, 0]));
    }
}
