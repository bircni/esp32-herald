//! Panel memory layouts: how a [`Frame`] is laid out in a display
//! controller's memory. Here rather than in the board crates so they are
//! tested on the host.

use super::Frame;

/// The `CrowPanel` 5.79" (two SSD1683 in cascade): 272 rows of 792 pixels,
/// 8 per byte.
pub const SSD1683_CASCADE_ROW_BYTES: usize = 99;
pub const SSD1683_CASCADE_ROWS: usize = 272;

/// A 272x792 upright frame in the `CrowPanel` 5.79"'s memory.
///
/// Rows of 792 pixels, a set bit is white, the first pixel of a row in the
/// lowest bit of its last byte (the panel scans mirrored). `upside_down`
/// turns the picture, for a board standing the other way round.
#[must_use]
pub fn ssd1683_cascade(frame: &Frame, upside_down: bool) -> Vec<u8> {
    const W: usize = SSD1683_CASCADE_ROWS;
    const H: usize = SSD1683_CASCADE_ROW_BYTES * 8;
    let mut out = vec![0xFF; SSD1683_CASCADE_ROW_BYTES * SSD1683_CASCADE_ROWS];
    for y in 0..H.min(frame.height() as usize) {
        for x in 0..W.min(frame.width() as usize) {
            if !frame.black(x as i32, y as i32) {
                continue;
            }
            // Upright x runs along the panel's rows, upright y along its
            // columns.
            let (col, row) = if upside_down {
                (H - 1 - y, x)
            } else {
                (y, W - 1 - x)
            };
            let mirrored = H - 1 - col;
            out[row * SSD1683_CASCADE_ROW_BYTES + mirrored / 8] &= !(0x80 >> (mirrored % 8));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_map_to_the_panel() {
        let mut f = Frame::blank(272, 792);
        f.set_black(0, 0);
        let p = ssd1683_cascade(&f, false);
        // Upright top-left: panel row 271, column 0, stored mirrored in the
        // last byte's lowest bit.
        assert_eq!(p[271 * SSD1683_CASCADE_ROW_BYTES + 98], !0x01);
        assert_eq!(p.iter().filter(|b| **b != 0xFF).count(), 1);
        let p = ssd1683_cascade(&f, true);
        assert_eq!(p[0], 0x7F, "upside down: panel row 0, column 791");
    }
}
