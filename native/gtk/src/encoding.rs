//! Encoding shared by the application and standalone updater.

/// Encode bytes as lowercase hexadecimal, preserving leading zeroes.
pub fn encode_hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    if bytes.is_empty() {
        return String::new();
    }
    let mut encoded = vec![0; bytes.len() * 2];
    for (&byte, digits) in bytes.iter().zip(encoded.chunks_exact_mut(2)) {
        digits[0] = DIGITS[usize::from(byte >> 4)];
        digits[1] = DIGITS[usize::from(byte & 0xf)];
    }
    String::from_utf8(encoded).expect("hexadecimal digits are ASCII")
}

#[cfg(test)]
mod tests {
    use super::encode_hex;

    #[test]
    fn hexadecimal_covers_every_byte_and_preserves_width() {
        assert_eq!(encode_hex([]), "");
        assert_eq!(encode_hex([0, 1, 15, 16, 128, 255]), "00010f1080ff");
        let bytes: Vec<u8> = (0..=255).collect();
        let expected: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(encode_hex(bytes), expected);
    }
}
