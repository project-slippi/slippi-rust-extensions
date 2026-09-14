/// Decodes a null-terminated Shift-JIS byte field. Names and connect codes
/// are mostly ASCII; the one common multi-byte case is the fullwidth `＃` in
/// connect codes, mapped to `#`. Other multi-byte sequences are skipped.
pub(crate) fn decode_shift_jis(field: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < field.len() {
        match field[i] {
            0x00 => break,
            0x81 if field.get(i + 1) == Some(&0x94) => {
                result.push('#');
                i += 2;
            },
            byte if byte.is_ascii() && !byte.is_ascii_control() => {
                result.push(byte as char);
                i += 1;
            },
            0x81..=0x9F | 0xE0..=0xFC => i += 2,
            _ => i += 1,
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::decode_shift_jis;

    #[test]
    fn decodes_ascii_and_fullwidth_hash() {
        assert_eq!(decode_shift_jis(b"SWOO\x81\x940\x00junk"), "SWOO#0");
        assert_eq!(decode_shift_jis(b"plain"), "plain");
        assert_eq!(decode_shift_jis(&[0x00]), "");
    }
}
