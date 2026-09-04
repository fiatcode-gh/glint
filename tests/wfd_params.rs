//! Round-trip tests for the four WFD parameter payloads.

use glint::wfd::params::{
    AudioCodecs, ClientRtpPorts, ContentProtection, ParamError, VideoFormats, WfdParam,
};

/// Parse a parameter value and format it straight back out.
type RoundTrip = fn(&str) -> String;

/// A canonical `wfd_video_formats` value: one H.264 codec entry, no maximum
/// resolution override.
const VIDEO_ONE_CODEC: &str = "40 00 02 04 0001ffff 1fffffff 00000fff 00 0000 0000 11 none none";

/// Two codec entries, comma-separated, the second carrying explicit maxima.
const VIDEO_TWO_CODECS: &str = "40 00 01 04 0001ffff 00000000 00000000 00 0000 0000 11 none none, \
02 08 0000ffff 1fffffff 00000fff 00 0010 0020 11 0780 0438";

const AUDIO_TWO: &str = "LPCM 00000002 00, AAC 00000001 00";
const AUDIO_NONE: &str = "none";
const PORTS: &str = "RTP/AVP/UDP;unicast 19000 0 mode=play";
const PROTECTION_NONE: &str = "none";
const PROTECTION_HDCP: &str = "HDCP2.0 port=1189";

/// `format(parse(s)) == s` for every canonical string.
#[test]
fn canonical_strings_round_trip_exactly() {
    // arrange
    let cases: Vec<(&str, RoundTrip)> = vec![
        (VIDEO_ONE_CODEC, |s| {
            VideoFormats::parse(s).unwrap().format()
        }),
        (VIDEO_TWO_CODECS, |s| {
            VideoFormats::parse(s).unwrap().format()
        }),
        (AUDIO_TWO, |s| AudioCodecs::parse(s).unwrap().format()),
        (AUDIO_NONE, |s| AudioCodecs::parse(s).unwrap().format()),
        (PORTS, |s| ClientRtpPorts::parse(s).unwrap().format()),
        (PROTECTION_NONE, |s| {
            ContentProtection::parse(s).unwrap().format()
        }),
        (PROTECTION_HDCP, |s| {
            ContentProtection::parse(s).unwrap().format()
        }),
    ];
    for (input, round_trip) in cases {
        // act
        let output = round_trip(input);
        // assert
        assert_eq!(output, input, "round trip changed the string");
    }
}

/// The weaker property, over deliberately non-canonical input: uppercase hex.
/// A real sink's formatting is sloppier than the canonical form, and parsing
/// must be stable under re-formatting even when the text is not.
#[test]
fn non_canonical_input_is_stable_under_reparsing() {
    // arrange
    let upper = "40 00 02 04 0001FFFF 1FFFFFFF 00000FFF 00 0000 0000 11 NONE NONE";
    // act
    let once = VideoFormats::parse(upper).unwrap();
    let twice = VideoFormats::parse(&once.format()).unwrap();
    // assert
    assert_eq!(twice, once);
    assert_eq!(
        once.format(),
        VIDEO_ONE_CODEC,
        "should normalise to canonical"
    );
}

#[test]
fn video_formats_keeps_every_codec_entry_and_its_order() {
    // act
    let parsed = VideoFormats::parse(VIDEO_TWO_CODECS).unwrap();
    // assert
    assert_eq!(parsed.codecs.len(), 2);
    assert_eq!(parsed.codecs[0].profile, 0x01);
    assert_eq!(parsed.codecs[1].profile, 0x02);
    assert_eq!(parsed.codecs[1].max_hres, Some(0x0780));
    assert_eq!(parsed.codecs[1].max_vres, Some(0x0438));
}

#[test]
fn video_formats_reads_the_leading_native_and_preferred_fields() {
    // act
    let parsed = VideoFormats::parse(VIDEO_ONE_CODEC).unwrap();
    // assert
    assert_eq!(parsed.native, 0x40);
    assert_eq!(parsed.preferred_display_mode, 0x00);
}

#[test]
fn audio_codecs_none_parses_to_an_empty_list() {
    // act
    let parsed = AudioCodecs::parse(AUDIO_NONE).unwrap();
    // assert
    assert!(parsed.0.is_empty());
}

#[test]
fn an_unknown_audio_format_name_parses_without_error() {
    // act
    let parsed = AudioCodecs::parse("FLAC 00000001 00").unwrap();
    // assert
    assert_eq!(parsed.0.len(), 1);
    assert_eq!(parsed.format(), "FLAC 00000001 00");
}

#[test]
fn content_protection_none_is_not_hdcp() {
    // act
    let parsed = ContentProtection::parse(PROTECTION_NONE).unwrap();
    // assert
    assert!(!parsed.requires_hdcp());
}

#[test]
fn content_protection_hdcp_is_detected_with_its_port() {
    // act
    let parsed = ContentProtection::parse(PROTECTION_HDCP).unwrap();
    // assert
    assert!(parsed.requires_hdcp());
}

#[test]
fn hdcp_two_point_one_is_also_detected() {
    // act
    let parsed = ContentProtection::parse("HDCP2.1 port=1189").unwrap();
    // assert
    assert!(parsed.requires_hdcp());
}

#[test]
fn client_rtp_ports_reads_both_ports() {
    // act
    let parsed = ClientRtpPorts::parse(PORTS).unwrap();
    // assert
    assert_eq!(parsed.rtp_port0, 19000);
    assert_eq!(parsed.rtp_port1, 0);
}

#[test]
fn an_rtp_port_with_a_leading_sign_is_rejected() {
    // `u16::from_str` accepts "+19000", and the hex fields deliberately refuse
    // a sign, so the decimal port fields refuse one too.
    // act
    let result = ClientRtpPorts::parse("RTP/AVP/UDP;unicast +19000 0 mode=play");
    // assert
    assert!(
        matches!(result, Err(ParamError::NotNumeric { .. })),
        "got: {result:?}"
    );
}

#[test]
fn an_hdcp_port_with_a_leading_sign_is_rejected() {
    // act
    let result = ContentProtection::parse("HDCP2.0 port=+1189");
    // assert
    assert!(
        matches!(result, Err(ParamError::NotNumeric { .. })),
        "got: {result:?}"
    );
}

#[test]
fn a_malformed_video_formats_value_is_rejected() {
    // act
    let result = VideoFormats::parse("40 00 02");
    // assert
    assert!(result.is_err());
}

#[test]
fn a_non_hex_video_field_is_rejected() {
    // act
    let result =
        VideoFormats::parse("zz 00 02 04 0001ffff 1fffffff 00000fff 00 0000 0000 11 none none");
    // assert
    assert!(result.is_err());
}

/// The fixture a real television's M3 reply will supply.
///
/// The committed `tests/fixtures/m3_reply_video_formats.txt` is a synthetic
/// placeholder: it holds a byte-identical copy of `VIDEO_ONE_CODEC` above, so
/// running this test with `--ignored` passes and proves nothing at all.
/// Milestone 2, Task 24 replaces the file with a real television's M3 reply;
/// only from then does a green run mean anything. Until then the test stays
/// ignored and named, so the gap is visible instead of hidden.
#[test]
#[ignore = "the committed fixture is a synthetic placeholder; the real-TV M3 reply arrives with Milestone 2, Task 24"]
fn the_real_sink_reply_fixture_round_trips() {
    // arrange
    let fixture = include_str!("fixtures/m3_reply_video_formats.txt").trim();
    // act
    let parsed = VideoFormats::parse(fixture).unwrap();
    // assert
    assert_eq!(parsed.format(), fixture);
}
