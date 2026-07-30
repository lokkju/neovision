//! The CP437 repertoire, as a byte ↔ `char` mapping.
//!
//! A [`Cell`](crate::Cell) holds a CP437 byte, because that is what text-mode
//! video memory holds. Any host that draws to something other than a real
//! text-mode display — a terminal, a pixel framebuffer, a canvas — has to turn
//! that byte into a glyph, and this is the table it needs.
//!
//! The mapping follows IBM's original code page 437, including the graphic
//! symbols in `0x00..=0x1F` that ASCII reserves for control codes. `0x00` maps
//! to a space rather than `NUL`, so a zeroed buffer renders as blank.
//!
//! ```
//! use neovision_core::cp437;
//!
//! assert_eq!(cp437::to_char(0xC9), '╔');
//! assert_eq!(cp437::to_char(b'A'), 'A');
//! assert_eq!(cp437::from_char('░'), Some(0xB0));
//! assert_eq!(cp437::from_char('漢'), None);
//! ```

/// Every CP437 byte as its Unicode equivalent, indexed by byte value.
///
/// `TABLE[0]` is a space, not `U+0000`: a blank cell and a zeroed cell should
/// look the same.
pub const TABLE: [char; 256] = [
    // 0x00..0x0F — the graphic symbols ASCII reserves for control codes.
    ' ', '☺', '☻', '♥', '♦', '♣', '♠', '•', '◘', '○', '◙', '♂', '♀', '♪', '♫', '☼',
    // 0x10..0x1F
    '►', '◄', '↕', '‼', '¶', '§', '▬', '↨', '↑', '↓', '→', '←', '∟', '↔', '▲', '▼',
    // 0x20..0x2F
    ' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/',
    // 0x30..0x3F
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?',
    // 0x40..0x4F
    '@', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    // 0x50..0x5F
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '[', '\\', ']', '^', '_',
    // 0x60..0x6F
    '`', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    // 0x70..0x7F
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '{', '|', '}', '~', '⌂',
    // 0x80..0x8F
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å',
    // 0x90..0x9F
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    // 0xA0..0xAF
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»',
    // 0xB0..0xBF — shades and single/double box drawing.
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    // 0xC0..0xCF
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
    // 0xD0..0xDF
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    // 0xE0..0xEF
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
    // 0xF0..0xFF
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// The glyph a CP437 byte stands for. Total — every byte has one.
#[inline]
pub const fn to_char(byte: u8) -> char {
    TABLE[byte as usize]
}

/// The CP437 byte for a `char`, or `None` if CP437 cannot represent it.
///
/// Only `0x20..=0x7E` is shared with ASCII. `0x7F` is `'⌂'` here, not `DEL`,
/// and the bytes below `0x20` are graphic symbols — so the ASCII fast path
/// stops at `0x7E` and everything else is looked up. The scan starts at `0x01`
/// so that a space resolves to `0x20` rather than to the blank at `0x00`.
pub fn from_char(ch: char) -> Option<u8> {
    if ch == ' ' || ch.is_ascii_graphic() {
        return Some(ch as u8);
    }
    let mut i = 1usize;
    while i < 256 {
        if TABLE[i] == ch {
            return Some(i as u8);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_identity_over_its_whole_range() {
        for b in 0x20u8..=0x7E {
            assert_eq!(to_char(b), b as char, "byte {b:#04x}");
        }
    }

    #[test]
    fn zero_renders_as_blank_not_nul() {
        assert_eq!(to_char(0x00), ' ');
    }

    #[test]
    fn box_drawing_bytes_match_the_frame_glyphs() {
        // The bytes `BoxChars::DOUBLE` is built from.
        assert_eq!(to_char(0xC9), '╔');
        assert_eq!(to_char(0xBB), '╗');
        assert_eq!(to_char(0xC8), '╚');
        assert_eq!(to_char(0xBC), '╝');
        assert_eq!(to_char(0xCD), '═');
        assert_eq!(to_char(0xBA), '║');
        // ...and `BoxChars::SINGLE`.
        assert_eq!(to_char(0xDA), '┌');
        assert_eq!(to_char(0xBF), '┐');
        assert_eq!(to_char(0xC0), '└');
        assert_eq!(to_char(0xD9), '┘');
        assert_eq!(to_char(0xC4), '─');
        assert_eq!(to_char(0xB3), '│');
    }

    #[test]
    fn control_range_holds_graphic_symbols_not_control_codes() {
        assert_eq!(to_char(0x01), '☺');
        assert_eq!(to_char(0x0D), '♪');
        assert_eq!(to_char(0x1F), '▼');
    }

    #[test]
    fn from_char_inverts_to_char_for_every_byte() {
        for b in 0u8..=255 {
            let ch = to_char(b);
            let back = from_char(ch).expect("every table glyph maps back");
            // Duplicated glyphs (0x00/0x20 are both a space) resolve to the
            // lower byte, so compare through the glyph rather than the byte.
            assert_eq!(to_char(back), ch, "byte {b:#04x} round trip");
        }
    }

    #[test]
    fn space_round_trips_to_the_ascii_byte_not_the_zero_byte() {
        assert_eq!(from_char(' '), Some(0x20));
    }

    #[test]
    fn chars_outside_the_repertoire_are_rejected() {
        assert_eq!(from_char('漢'), None);
        assert_eq!(from_char('€'), None);
    }

    #[test]
    fn byte_7f_is_the_house_glyph_and_ascii_del_is_not_representable() {
        assert_eq!(to_char(0x7F), '⌂');
        assert_eq!(from_char('⌂'), Some(0x7F));
        assert_eq!(from_char('\u{7f}'), None);
    }

    #[test]
    fn the_table_is_exactly_the_byte_range() {
        assert_eq!(TABLE.len(), 256);
    }
}
