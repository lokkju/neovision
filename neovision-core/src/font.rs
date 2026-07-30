//! The IBM VGA 8×16 text-mode face, as glyph bitmaps.
//!
//! A [`Cell`](crate::Cell) holds a CP437 byte. A terminal host maps that byte
//! to a `char` with [`cp437`](crate::cp437) and lets the terminal find a glyph;
//! a host drawing actual pixels — a framebuffer, a canvas, a real VGA mode —
//! has to rasterize the glyph itself, and this is the face it needs.
//!
//! Each glyph is 16 rows of 8 pixels, one byte per row, most significant bit
//! leftmost. That is the layout of the font ROM in VGA hardware, so a row byte
//! can be blitted by walking its bits.
//!
//! ```
//! use neovision_core::font;
//!
//! // Row 7 of 'A' is its crossbar: ##...## with the two stems lit.
//! let a = font::glyph(b'A');
//! assert_eq!(a.len(), font::GLYPH_H as usize);
//! assert_eq!(a[7], 0b1111_1110);
//! ```
//!
//! # Using a different face
//!
//! Nothing here is privileged. A host that wants its own face declares its own
//! `[[u8; 16]; 256]` and blits from that instead — the toolkit never reads this
//! table, it only hands out cells. This one exists so that every pixel host does
//! not have to start by finding a font.
//!
//! # Provenance
//!
//! These bytes are a dump of an IBM VGA BIOS ROM character generator. Typeface
//! *designs* are not copyrightable subject matter in the United States
//! (37 CFR 202.1(e)), and a bitmap face like this one is data rather than a
//! program, which is why such dumps circulate freely. Scalable font *programs*
//! are a different matter and none are involved here.

/// Width of one glyph, in pixels. One byte per row, so this is always 8.
pub const GLYPH_W: u16 = 8;

/// Height of a [`VGA_8X16`] glyph, in pixels.
pub const GLYPH_H: u16 = 16;

/// Height of a [`VGA_8X8`] glyph, in pixels.
pub const GLYPH_H_8: u16 = 8;

/// Every CP437 glyph as 16 rows of 8 bits, indexed by byte value.
///
/// Bit 7 of a row is its leftmost pixel. A set bit is foreground, a clear bit
/// is background — the cell's attribute decides what those two colours are.
pub static VGA_8X16: [[u8; GLYPH_H as usize]; 256] = {
    let raw = *include_bytes!("vga_8x16.bin");
    let mut out = [[0u8; GLYPH_H as usize]; 256];
    let mut ch = 0usize;
    while ch < 256 {
        let mut row = 0usize;
        while row < GLYPH_H as usize {
            out[ch][row] = raw[ch * GLYPH_H as usize + row];
            row += 1;
        }
        ch += 1;
    }
    out
};

/// The same repertoire at 8x8, for displays that cannot spare the rows.
///
/// A 320x240 panel fits 40x15 cells at 8x16 but 40x30 at 8x8, which is the
/// difference between a form that fits and one that does not. The face is the
/// squarer, blockier one the CGA and EGA ROMs carried, not a squashed 8x16.
pub static VGA_8X8: [[u8; GLYPH_H_8 as usize]; 256] = {
    let raw = *include_bytes!("vga_8x8.bin");
    let mut out = [[0u8; GLYPH_H_8 as usize]; 256];
    let mut ch = 0usize;
    while ch < 256 {
        let mut row = 0usize;
        while row < GLYPH_H_8 as usize {
            out[ch][row] = raw[ch * GLYPH_H_8 as usize + row];
            row += 1;
        }
        ch += 1;
    }
    out
};

/// The 8x8 glyph for a CP437 byte.
#[inline]
pub fn glyph_8x8(ch: u8) -> &'static [u8; GLYPH_H_8 as usize] {
    &VGA_8X8[ch as usize]
}

/// The glyph for a CP437 byte. Total — every byte has one.
///
/// Not a `const fn`: it hands out a reference into [`VGA_8X16`], and a const
/// context may not refer to a `static`. Callers wanting the data at compile
/// time can index the table directly.
#[inline]
pub fn glyph(ch: u8) -> &'static [u8; GLYPH_H as usize] {
    &VGA_8X16[ch as usize]
}

/// Whether the pixel at (`x`, `y`) within a glyph is foreground.
///
/// Out-of-range coordinates read as background rather than panicking, so a
/// blitter may run its own loop bounds without pre-clamping.
#[inline]
pub fn pixel(ch: u8, x: u16, y: u16) -> bool {
    if x >= GLYPH_W || y >= GLYPH_H {
        return false;
    }
    let row = VGA_8X16[ch as usize][y as usize];
    (row >> (GLYPH_W - 1 - x)) & 1 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_small_face_covers_every_byte_at_its_own_size() {
        assert_eq!(VGA_8X8.len(), 256);
        assert_eq!(VGA_8X8[0].len(), GLYPH_H_8 as usize);
    }

    #[test]
    fn the_small_face_is_a_different_face_not_a_squashed_one() {
        // If it were derived by dropping rows from the 8x16 face, 'A' would
        // share its crossbar row. It does not: it is the CGA/EGA ROM face.
        assert_ne!(glyph_8x8(b'A')[..], VGA_8X16[b'A' as usize][..8]);
        // ...but it is still recognisably CP437: space blank, block solid.
        assert!(glyph_8x8(b' ').iter().all(|&r| r == 0x00));
        assert!(glyph_8x8(0xDB).iter().all(|&r| r == 0xFF));
    }

    #[test]
    fn the_table_covers_every_byte_at_the_declared_size() {
        assert_eq!(VGA_8X16.len(), 256);
        assert_eq!(VGA_8X16[0].len(), GLYPH_H as usize);
        assert_eq!(GLYPH_W, 8);
    }

    #[test]
    fn space_is_blank_and_full_block_is_solid() {
        assert!(glyph(b' ').iter().all(|&r| r == 0x00));
        // 0xDB is the full block; every row is lit edge to edge.
        assert!(glyph(0xDB).iter().all(|&r| r == 0xFF));
    }

    #[test]
    fn capital_a_has_a_full_width_crossbar() {
        // Row 7 of 'A' is the crossbar joining both stems.
        assert_eq!(glyph(b'A')[7], 0b1111_1110);
    }

    #[test]
    fn the_vertical_box_rule_is_a_single_centred_stem() {
        // 0xB3 is │ — the same two-pixel stem on every row.
        let g = glyph(0xB3);
        assert!(g.iter().all(|&r| r == 0x00 || r == 0b0001_1000));
        assert!(g.contains(&0b0001_1000));
    }

    #[test]
    fn the_light_shade_alternates_between_two_row_patterns() {
        // 0xB0 is ░ — a dither of two interleaved row patterns.
        let g = glyph(0xB0);
        for (i, &row) in g.iter().enumerate() {
            let expected = if i % 2 == 0 { 0b0001_0001 } else { 0b0100_0100 };
            assert_eq!(row, expected, "row {i}");
        }
    }

    #[test]
    fn pixel_agrees_with_the_raw_row_bits() {
        for x in 0..GLYPH_W {
            let lit = (glyph(b'A')[7] >> (GLYPH_W - 1 - x)) & 1 != 0;
            assert_eq!(pixel(b'A', x, 7), lit, "column {x}");
        }
    }

    #[test]
    fn pixel_reads_out_of_range_coordinates_as_background() {
        assert!(!pixel(0xDB, GLYPH_W, 0));
        assert!(!pixel(0xDB, 0, GLYPH_H));
    }
}
