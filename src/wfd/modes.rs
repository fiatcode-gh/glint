//! The Wi-Fi Display resolution/refresh-rate tables.
//!
//! `wfd_video_formats` carries three support bitmaps — CEA, VESA and Handheld.
//! Each bit means "this row of the corresponding table is supported". The
//! tables below are the bit-to-mode mapping.
//!
//! Provenance: transcribed clean-room from the Wi-Fi Display specification,
//! with GNOME Network Displays' `src/wfd/wfd-params.c` read as the worked
//! reference (never vendored — GND is GPL and glint is a clean-room Rust
//! implementation). **These rows are doc-sourced and have not been verified
//! against any real sink.** The first real-TV capture (Milestone 2, Task 24)
//! is the first chance to check them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoMode {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Kept so that parsing a sink's bitmap loses nothing. Negotiation filters
    /// interlaced modes out of the candidate set — a desktop capture source is
    /// progressive, and 1080i60 would otherwise tie with 1080p60 under the
    /// `(area, fps)` ordering key.
    pub interlaced: bool,
}

const fn p(width: u32, height: u32, fps: u32) -> VideoMode {
    VideoMode { width, height, fps, interlaced: false }
}

const fn i(width: u32, height: u32, fps: u32) -> VideoMode {
    VideoMode { width, height, fps, interlaced: true }
}

/// CEA timings, bits 0..=16 of the CEA support bitmap.
pub const CEA_MODES: [VideoMode; 17] = [
    p(640, 480, 60),    // 0
    p(720, 480, 60),    // 1
    i(720, 480, 60),    // 2
    p(720, 576, 50),    // 3
    i(720, 576, 50),    // 4
    p(1280, 720, 30),   // 5
    p(1280, 720, 60),   // 6
    p(1920, 1080, 30),  // 7
    p(1920, 1080, 60),  // 8
    i(1920, 1080, 60),  // 9
    p(1280, 720, 25),   // 10
    p(1280, 720, 50),   // 11
    p(1920, 1080, 25),  // 12
    p(1920, 1080, 50),  // 13
    i(1920, 1080, 50),  // 14
    p(1280, 720, 24),   // 15
    p(1920, 1080, 24),  // 16
];

/// VESA timings, bits 0..=28 of the VESA support bitmap.
pub const VESA_MODES: [VideoMode; 29] = [
    p(800, 600, 30),    // 0
    p(800, 600, 60),    // 1
    p(1024, 768, 30),   // 2
    p(1024, 768, 60),   // 3
    p(1152, 864, 30),   // 4
    p(1152, 864, 60),   // 5
    p(1280, 768, 30),   // 6
    p(1280, 768, 60),   // 7
    p(1280, 800, 30),   // 8
    p(1280, 800, 60),   // 9
    p(1360, 768, 30),   // 10
    p(1360, 768, 60),   // 11
    p(1366, 768, 30),   // 12
    p(1366, 768, 60),   // 13
    p(1280, 1024, 30),  // 14
    p(1280, 1024, 60),  // 15
    p(1400, 1050, 30),  // 16
    p(1400, 1050, 60),  // 17
    p(1440, 900, 30),   // 18
    p(1440, 900, 60),   // 19
    p(1600, 900, 30),   // 20
    p(1600, 900, 60),   // 21
    p(1600, 1200, 30),  // 22
    p(1600, 1200, 60),  // 23
    p(1680, 1024, 30),  // 24
    p(1680, 1024, 60),  // 25
    p(1680, 1050, 30),  // 26
    p(1680, 1050, 60),  // 27
    p(1920, 1200, 30),  // 28
];

/// Handheld timings, bits 0..=11 of the HH support bitmap.
pub const HH_MODES: [VideoMode; 12] = [
    p(800, 480, 30),   // 0
    p(800, 480, 60),   // 1
    p(854, 480, 30),   // 2
    p(854, 480, 60),   // 3
    p(864, 480, 30),   // 4
    p(864, 480, 60),   // 5
    p(640, 360, 30),   // 6
    p(640, 360, 60),   // 7
    p(960, 540, 30),   // 8
    p(960, 540, 60),   // 9
    p(848, 480, 30),   // 10
    p(848, 480, 60),   // 11
];

/// The modes a support bitmap selects from a table. Bits with no row are
/// ignored: a newer sink may set bits this table does not know.
pub fn modes_for_mask(mask: u32, table: &'static [VideoMode]) -> impl Iterator<Item = VideoMode> {
    table
        .iter()
        .enumerate()
        .filter(move |(bit, _)| mask & (1u32 << bit) != 0)
        .map(|(_, mode)| *mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_tables_have_the_lengths_the_wfd_bitmaps_imply() {
        // The all-supported masks seen from real sinks are 0x0001ffff (CEA),
        // 0x1fffffff (VESA) and 0x00000fff (HH) — 17, 29 and 12 bits.
        // act & assert
        assert_eq!(CEA_MODES.len(), 17);
        assert_eq!(VESA_MODES.len(), 29);
        assert_eq!(HH_MODES.len(), 12);
    }

    #[test]
    fn cea_bit_zero_is_vga_and_bit_eight_is_1080p60() {
        // act & assert
        assert_eq!(
            CEA_MODES[0],
            VideoMode { width: 640, height: 480, fps: 60, interlaced: false }
        );
        assert_eq!(
            CEA_MODES[8],
            VideoMode { width: 1920, height: 1080, fps: 60, interlaced: false }
        );
    }

    #[test]
    fn exactly_four_cea_modes_are_interlaced() {
        // 720x480i60, 720x576i50, 1920x1080i60, 1920x1080i50.
        // act
        let interlaced: Vec<&VideoMode> = CEA_MODES.iter().filter(|m| m.interlaced).collect();
        // assert
        assert_eq!(interlaced.len(), 4);
    }

    #[test]
    fn no_vesa_or_handheld_mode_is_interlaced() {
        // act & assert
        assert!(VESA_MODES.iter().all(|m| !m.interlaced));
        assert!(HH_MODES.iter().all(|m| !m.interlaced));
    }

    #[test]
    fn modes_for_mask_returns_only_the_set_bits() {
        // arrange: bits 0 and 8 of the CEA mask
        let mask = (1 << 0) | (1 << 8);
        // act
        let modes: Vec<VideoMode> = modes_for_mask(mask, &CEA_MODES).collect();
        // assert
        assert_eq!(modes, vec![CEA_MODES[0], CEA_MODES[8]]);
    }

    #[test]
    fn modes_for_mask_ignores_bits_beyond_the_table() {
        // arrange: bit 31 has no CEA row
        let mask = (1u32 << 31) | (1 << 0);
        // act
        let modes: Vec<VideoMode> = modes_for_mask(mask, &CEA_MODES).collect();
        // assert
        assert_eq!(modes, vec![CEA_MODES[0]]);
    }

    #[test]
    fn every_table_row_has_a_plausible_shape() {
        // A typo guard: no zero dimension, no zero frame rate.
        for table in [CEA_MODES.as_slice(), VESA_MODES.as_slice(), HH_MODES.as_slice()] {
            for mode in table {
                // assert
                assert!(mode.width > 0 && mode.height > 0, "{mode:?}");
                assert!(mode.fps > 0, "{mode:?}");
            }
        }
    }
}
