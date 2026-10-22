//! Colour entry for the swatch: the hex field.

use ssmap_core::Color;

/// "#RRGGBB" for the hex field.
pub(super) fn hex_text(colour: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", colour.r, colour.g, colour.b)
}

/// Parses what was typed in the hex field. Accepts `#abc`, `#aabbcc` (keeping `alpha`) and
/// `#aabbccdd`, with or without the `#`.
pub fn parse_hex(text: &str, alpha: u8) -> Option<Color> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
    match digits.len() {
        3 => Some(Color::rgba(
            byte(0..1)? * 17,
            byte(1..2)? * 17,
            byte(2..3)? * 17,
            alpha,
        )),
        6 => Some(Color::rgba(byte(0..2)?, byte(2..4)?, byte(4..6)?, alpha)),
        8 => Some(Color::rgba(
            byte(0..2)?,
            byte(2..4)?,
            byte(4..6)?,
            byte(6..8)?,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_field_parses_short_and_long_forms() {
        assert_eq!(parse_hex("#fff", 255), Some(Color::rgb(255, 255, 255)));
        assert_eq!(parse_hex("c8a03c", 10), Some(Color::rgba(200, 160, 60, 10)));
        assert_eq!(
            parse_hex("#c8a03c80", 255),
            Some(Color::rgba(200, 160, 60, 128))
        );
        assert_eq!(parse_hex("#12", 255), None);
        assert_eq!(parse_hex("#ggg", 255), None);
    }
}
