//! Panel memory layouts: how a [`Frame`] is laid out in a display
//! controller's memory. Here rather than in the board crates so they are
//! tested on the host.

use super::Frame;

/// The `CrowPanel` 5.79" (two SSD1683 in cascade): 272 rows of 792 pixels,
/// 8 per byte.
pub const SSD1683_CASCADE_ROW_BYTES: usize = 99;
pub const SSD1683_CASCADE_ROWS: usize = 272;

/// A frame in the `CrowPanel` 5.79"'s memory: 272x792 upright or 792x272
/// in landscape.
///
/// Rows of 792 pixels, a set bit is white, the first pixel of a row in the
/// lowest bit of its last byte (the panel scans mirrored). `upside_down`
/// turns the picture, for a board standing the other way round.
#[must_use]
pub fn ssd1683_cascade(frame: &Frame, upside_down: bool) -> Vec<u8> {
    // The panel's own size: 792 columns, 272 rows.
    const COLS: usize = SSD1683_CASCADE_ROW_BYTES * 8;
    const ROWS: usize = SSD1683_CASCADE_ROWS;
    let mut out = vec![0xFF; SSD1683_CASCADE_ROW_BYTES * SSD1683_CASCADE_ROWS];
    let landscape = frame.width() > frame.height();
    let (w, h) = if landscape {
        (COLS, ROWS)
    } else {
        (ROWS, COLS)
    };
    for y in 0..h.min(frame.height() as usize) {
        for x in 0..w.min(frame.width() as usize) {
            if !frame.black(x as i32, y as i32) {
                continue;
            }
            let (col, row) = match (landscape, upside_down) {
                // Landscape is the panel's own way round.
                (true, false) => (x, y),
                (true, true) => (COLS - 1 - x, ROWS - 1 - y),
                // Upright x runs along the panel's rows, y along its
                // columns.
                (false, false) => (y, ROWS - 1 - x),
                (false, true) => (COLS - 1 - y, x),
            };
            let mirrored = COLS - 1 - col;
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

        let mut f = Frame::blank(792, 272);
        f.set_black(0, 0);
        let p = ssd1683_cascade(&f, false);
        assert_eq!(p[98], !0x01, "landscape: panel row 0, column 0");
        assert_eq!(p.iter().filter(|b| **b != 0xFF).count(), 1);
        let p = ssd1683_cascade(&f, true);
        assert_eq!(p[271 * SSD1683_CASCADE_ROW_BYTES], 0x7F);
    }
}
