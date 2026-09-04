//! Per-vendor workarounds, keyed on the sink's WFD device-info string.
//!
//! The table is deliberately almost empty: real entries are earned by a real
//! television misbehaving, not guessed in advance. The one entry present is
//! fictional and exists so the lookup path itself is tested.

use crate::receiver::Resolution;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quirks {
    /// Override the negotiated resolution for sinks that advertise more than
    /// they can actually decode.
    pub force_resolution: Option<Resolution>,
    /// Drop the audio branch entirely for sinks that stall on it.
    pub ignore_audio: bool,
    /// Pause this long between RTSP messages for sinks that race.
    pub extra_rtsp_delay_ms: u32,
}

/// One row: a lowercase substring to look for, and the quirks it implies.
struct QuirkEntry {
    device_info_contains: &'static str,
    quirks: Quirks,
}

const QUIRKS_TABLE: &[QuirkEntry] = &[QuirkEntry {
    // Fictional. Real vendors arrive as real bugs are found.
    device_info_contains: "glinttest reference sink",
    quirks: Quirks {
        force_resolution: Some(Resolution { width: 1280, height: 720 }),
        ignore_audio: true,
        extra_rtsp_delay_ms: 250,
    },
}];

/// The quirks for a sink, or the inert defaults when nothing matches.
pub fn quirks_for(device_info: &str) -> Quirks {
    let haystack = device_info.to_ascii_lowercase();
    QUIRKS_TABLE
        .iter()
        .find(|entry| haystack.contains(entry.device_info_contains))
        .map(|entry| entry.quirks)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_device_gets_the_defaults() {
        // act
        let quirks = quirks_for("Some Television 9000");
        // assert
        assert_eq!(quirks, Quirks::default());
    }

    #[test]
    fn the_defaults_are_all_inert() {
        // act
        let quirks = Quirks::default();
        // assert
        assert_eq!(quirks.force_resolution, None);
        assert!(!quirks.ignore_audio);
        assert_eq!(quirks.extra_rtsp_delay_ms, 0);
    }

    #[test]
    fn the_test_vendor_entry_is_returned_for_a_matching_device() {
        // act
        let quirks = quirks_for("GlintTest Reference Sink v1");
        // assert
        assert_eq!(
            quirks.force_resolution,
            Some(Resolution { width: 1280, height: 720 })
        );
        assert!(quirks.ignore_audio);
        assert_eq!(quirks.extra_rtsp_delay_ms, 250);
    }

    #[test]
    fn matching_is_a_case_insensitive_substring_test() {
        // Device-info strings from real sinks carry extra vendor text around
        // the identifying part, so the table matches on a substring.
        // act
        let quirks = quirks_for("acme glinttest reference sink v1 (build 42)");
        // assert
        assert!(quirks.ignore_audio);
    }

    #[test]
    fn an_empty_device_info_gets_the_defaults() {
        // act & assert
        assert_eq!(quirks_for(""), Quirks::default());
    }
}
