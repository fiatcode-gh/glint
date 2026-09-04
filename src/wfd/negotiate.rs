//! Capability negotiation: our formats plus the sink's, into one chosen format.

use crate::wfd::modes::{CEA_MODES, HH_MODES, VESA_MODES, modes_for_mask};
use crate::wfd::params::{ContentProtection, VideoFormats};

/// The H.264 profiles Wi-Fi Display defines, as bits of the profile field.
/// Constrained Baseline is the profile every sink must support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum H264Profile {
    ConstrainedBaseline,
    ConstrainedHigh,
}

impl H264Profile {
    const CBP_BIT: u8 = 0x01;
    const CHP_BIT: u8 = 0x02;

    /// Ordering rank — higher is better. Derived `Ord` already gives this
    /// because the variants are declared low to high, but naming it keeps the
    /// choice key readable.
    fn rank(self) -> u8 {
        match self {
            H264Profile::ConstrainedBaseline => 0,
            H264Profile::ConstrainedHigh => 1,
        }
    }
}

/// The best profile both sides support, if any.
fn common_profile(ours: u8, theirs: u8) -> Option<H264Profile> {
    let common = ours & theirs;
    if common & H264Profile::CHP_BIT != 0 {
        Some(H264Profile::ConstrainedHigh)
    } else if common & H264Profile::CBP_BIT != 0 {
        Some(H264Profile::ConstrainedBaseline)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChosenFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub profile: H264Profile,
}

impl ChosenFormat {
    /// The choice key from decision D11: highest common resolution, then
    /// highest frame rate, then highest profile. Profile breaks ties **last**.
    fn key(&self) -> (u32, u32, u8) {
        (self.width * self.height, self.fps, self.profile.rank())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NegotiationError {
    #[error("the source and the sink share no video format")]
    NoCommonFormat,
    #[error("the sink demands HDCP content protection, which Linux cannot provide")]
    HdcpRequired,
}

/// Choose the format to stream.
///
/// The sink's `wfd_content_protection` is a third argument because it is a
/// separate WFD parameter: neither of the two format arguments can carry it,
/// and the HDCP refusal has to happen before any matching so the error names
/// the real reason rather than a misleading `NoCommonFormat`.
///
/// Interlaced modes are dropped from the candidate set. A desktop capture
/// source is progressive, so offering interlaced would be a lie — and 1080i60
/// would otherwise tie with 1080p60 under the choice key, making the result
/// depend on table iteration order.
///
/// A codec entry's `max_hres`/`max_vres` are parsed by `params.rs` and
/// deliberately not consulted here: the choice key is exactly the one the
/// specification defines, so a sink that sets a mode bit above its own
/// declared maximum still gets that mode chosen — `Quirks::force_resolution`
/// is where that class of sink is dealt with.
pub fn negotiate(
    ours: &VideoFormats,
    sink_video_formats: &VideoFormats,
    sink_protection: &ContentProtection,
) -> Result<ChosenFormat, NegotiationError> {
    if sink_protection.requires_hdcp() {
        return Err(NegotiationError::HdcpRequired);
    }

    let mut best: Option<ChosenFormat> = None;

    for our_codec in &ours.codecs {
        for their_codec in &sink_video_formats.codecs {
            let Some(profile) = common_profile(our_codec.profile, their_codec.profile) else {
                continue;
            };

            let bitmaps = [
                (our_codec.cea & their_codec.cea, CEA_MODES.as_slice()),
                (our_codec.vesa & their_codec.vesa, VESA_MODES.as_slice()),
                (our_codec.hh & their_codec.hh, HH_MODES.as_slice()),
            ];

            for (mask, table) in bitmaps {
                for mode in modes_for_mask(mask, table) {
                    if mode.interlaced {
                        continue;
                    }
                    let candidate = ChosenFormat {
                        width: mode.width,
                        height: mode.height,
                        fps: mode.fps,
                        profile,
                    };
                    if best.is_none_or(|current| candidate.key() > current.key()) {
                        best = Some(candidate);
                    }
                }
            }
        }
    }

    best.ok_or(NegotiationError::NoCommonFormat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wfd::params::H264Codec;

    /// A codec entry with the given profile and support bitmaps.
    fn codec(profile: u8, cea: u32, vesa: u32, hh: u32) -> H264Codec {
        H264Codec {
            profile,
            level: 0x04,
            cea,
            vesa,
            hh,
            latency: 0,
            min_slice_size: 0,
            slice_enc_params: 0,
            frame_rate_control: 0x11,
            max_hres: None,
            max_vres: None,
        }
    }

    fn formats(codecs: Vec<H264Codec>) -> VideoFormats {
        VideoFormats {
            native: 0x40,
            preferred_display_mode: 0x00,
            codecs,
        }
    }

    const CBP: u8 = 0x01;
    const CHP: u8 = 0x02;

    /// CEA bit 8 is 1920x1080p60, bit 6 is 1280x720p60, bit 9 is 1080i60.
    const CEA_1080P60: u32 = 1 << 8;
    const CEA_720P60: u32 = 1 << 6;
    const CEA_1080I60: u32 = 1 << 9;

    #[test]
    fn the_derived_profile_ordering_and_rank_agree() {
        // `key()` orders profiles by `rank()` while the enum also derives
        // `Ord`; nothing else pins the two hand-written orders to each other.
        // act & assert
        assert!(H264Profile::ConstrainedBaseline < H264Profile::ConstrainedHigh);
        assert!(H264Profile::ConstrainedBaseline.rank() < H264Profile::ConstrainedHigh.rank());
    }

    #[test]
    fn the_highest_common_resolution_wins() {
        // arrange
        let ours = formats(vec![codec(CBP, CEA_1080P60 | CEA_720P60, 0, 0)]);
        let theirs = formats(vec![codec(CBP, CEA_1080P60 | CEA_720P60, 0, 0)]);
        // act
        let chosen = negotiate(&ours, &theirs, &ContentProtection::None).unwrap();
        // assert
        assert_eq!(chosen.width, 1920);
        assert_eq!(chosen.height, 1080);
        assert_eq!(chosen.fps, 60);
    }

    #[test]
    fn only_modes_both_sides_support_are_candidates() {
        // arrange: we do 1080p60 and 720p60, the sink only 720p60
        let ours = formats(vec![codec(CBP, CEA_1080P60 | CEA_720P60, 0, 0)]);
        let theirs = formats(vec![codec(CBP, CEA_720P60, 0, 0)]);
        // act
        let chosen = negotiate(&ours, &theirs, &ContentProtection::None).unwrap();
        // assert
        assert_eq!((chosen.width, chosen.height, chosen.fps), (1280, 720, 60));
    }

    #[test]
    fn frame_rate_breaks_a_resolution_tie() {
        // arrange: CEA bit 7 is 1080p30, bit 8 is 1080p60
        let both = formats(vec![codec(CBP, (1 << 7) | (1 << 8), 0, 0)]);
        // act
        let chosen = negotiate(&both, &both, &ContentProtection::None).unwrap();
        // assert
        assert_eq!(chosen.fps, 60);
    }

    #[test]
    fn the_highest_common_profile_is_chosen() {
        // arrange: both sides offer the same mode under CBP and CHP
        let ours = formats(vec![codec(CBP | CHP, CEA_1080P60, 0, 0)]);
        let theirs = formats(vec![codec(CBP | CHP, CEA_1080P60, 0, 0)]);
        // act
        let chosen = negotiate(&ours, &theirs, &ContentProtection::None).unwrap();
        // assert
        assert_eq!(chosen.profile, H264Profile::ConstrainedHigh);
    }

    #[test]
    fn profile_breaks_ties_last_not_first() {
        // arrange: the sink's CHP entry only reaches 720p60, its CBP entry
        // reaches 1080p60. Resolution dominates, so CBP at 1080p60 must win
        // even though CHP is the higher profile.
        let ours = formats(vec![codec(CBP | CHP, CEA_1080P60 | CEA_720P60, 0, 0)]);
        let theirs = formats(vec![
            codec(CBP, CEA_1080P60, 0, 0),
            codec(CHP, CEA_720P60, 0, 0),
        ]);
        // act
        let chosen = negotiate(&ours, &theirs, &ContentProtection::None).unwrap();
        // assert
        assert_eq!((chosen.width, chosen.height), (1920, 1080));
        assert_eq!(chosen.profile, H264Profile::ConstrainedBaseline);
    }

    #[test]
    fn interlaced_modes_are_never_chosen() {
        // arrange: 1080i60 has the same area and frame rate as 1080p60, so
        // without the filter this would be a nondeterministic tie. Here only
        // the interlaced bit is common, so there is no common progressive mode.
        let ours = formats(vec![codec(CBP, CEA_1080I60, 0, 0)]);
        let theirs = formats(vec![codec(CBP, CEA_1080I60, 0, 0)]);
        // act
        let result = negotiate(&ours, &theirs, &ContentProtection::None);
        // assert
        assert_eq!(result, Err(NegotiationError::NoCommonFormat));
    }

    #[test]
    fn vesa_and_handheld_modes_are_candidates_too() {
        // arrange: VESA bit 28 is 1920x1200p30 — bigger area than any CEA mode
        let ours = formats(vec![codec(CBP, CEA_1080P60, 1 << 28, 0)]);
        let theirs = formats(vec![codec(CBP, CEA_1080P60, 1 << 28, 0)]);
        // act
        let chosen = negotiate(&ours, &theirs, &ContentProtection::None).unwrap();
        // assert
        assert_eq!((chosen.width, chosen.height, chosen.fps), (1920, 1200, 30));
    }

    #[test]
    fn a_handheld_mode_wins_when_it_is_the_only_overlap() {
        // arrange: HH bit 9 is 960x540p60, and it is the only bit both sides share
        let ours = formats(vec![codec(CBP, 0, 0, 1 << 9)]);
        let theirs = formats(vec![codec(CBP, 0, 0, 1 << 9)]);
        // act
        let chosen = negotiate(&ours, &theirs, &ContentProtection::None).unwrap();
        // assert
        assert_eq!((chosen.width, chosen.height, chosen.fps), (960, 540, 60));
    }

    #[test]
    fn no_overlap_at_all_is_no_common_format() {
        // arrange
        let ours = formats(vec![codec(CBP, CEA_1080P60, 0, 0)]);
        let theirs = formats(vec![codec(CBP, CEA_720P60, 0, 0)]);
        // act
        let result = negotiate(&ours, &theirs, &ContentProtection::None);
        // assert
        assert_eq!(result, Err(NegotiationError::NoCommonFormat));
    }

    #[test]
    fn no_common_profile_is_no_common_format() {
        // arrange: same modes, disjoint profiles
        let ours = formats(vec![codec(CBP, CEA_1080P60, 0, 0)]);
        let theirs = formats(vec![codec(CHP, CEA_1080P60, 0, 0)]);
        // act
        let result = negotiate(&ours, &theirs, &ContentProtection::None);
        // assert
        assert_eq!(result, Err(NegotiationError::NoCommonFormat));
    }

    #[test]
    fn hdcp_is_refused_before_any_format_matching() {
        // arrange: a perfectly matching pair, but the sink demands HDCP
        let both = formats(vec![codec(CBP, CEA_1080P60, 0, 0)]);
        let protection = ContentProtection::Hdcp {
            version: "HDCP2.0".to_string(),
            port: 1189,
        };
        // act
        let result = negotiate(&both, &both, &protection);
        // assert
        assert_eq!(result, Err(NegotiationError::HdcpRequired));
    }

    #[test]
    fn hdcp_is_refused_even_when_no_format_would_have_matched() {
        // The HDCP check must come first, so the error names the real reason
        // rather than a misleading NoCommonFormat.
        // arrange
        let ours = formats(vec![codec(CBP, CEA_1080P60, 0, 0)]);
        let theirs = formats(vec![codec(CBP, CEA_720P60, 0, 0)]);
        let protection = ContentProtection::Hdcp {
            version: "HDCP2.1".to_string(),
            port: 1189,
        };
        // act
        let result = negotiate(&ours, &theirs, &protection);
        // assert
        assert_eq!(result, Err(NegotiationError::HdcpRequired));
    }

    #[test]
    fn an_empty_codec_list_is_no_common_format() {
        // arrange
        let empty = formats(vec![]);
        let ours = formats(vec![codec(CBP, CEA_1080P60, 0, 0)]);
        // act
        let result = negotiate(&ours, &empty, &ContentProtection::None);
        // assert
        assert_eq!(result, Err(NegotiationError::NoCommonFormat));
    }
}
