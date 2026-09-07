//! The Wi-Fi Display M1-M16 flow, as pure functions.
//!
//! Nothing here touches a socket or reads a clock. Message builders return the
//! bytes that go on the wire, so a test asserts exactly what a sink would
//! receive; the flow state machine takes elapsed time as an argument, so every
//! timer is arithmetic a test can pin without a clock. `wfd::rtsp` is the thin
//! shell that moves those bytes and supplies that elapsed time.
//!
//! Protocol behaviour is copied from GNOME Network Displays, which is
//! field-proven against real sinks, and cross-checked against the
//! miraclecast-family sink. Where the two references and the specification
//! disagree, the field behaviour wins and the reason is written down at the
//! point of the decision.

use std::net::SocketAddr;

use rtsp_types::{Message, Method, Request, Url, Version, headers};

use crate::wfd::modes::{Table, bit_for_mode};
use crate::wfd::negotiate::{ChosenFormat, H264Profile};
use crate::wfd::params::{
    AudioCodec, AudioCodecs, ClientRtpPorts, H264Codec, VideoFormats, WfdParam,
};

/// The request URI of M3, M4, M5 and M16. Literally `localhost`: no sink ever
/// resolves it, and it is what both reference implementations send.
const WFD_URI: &str = "rtsp://localhost/wfd1.0";

/// The Wi-Fi Display profile token — M1's `Require`, and the first entry of
/// the M2 reply's `Public`.
const WFD_PROFILE: &str = "org.wfa.wfd1.0";

fn wfd_uri() -> Url {
    // The literal is a constant this crate owns, so a parse failure would be a
    // bug in that literal rather than anything a peer can provoke.
    Url::parse(WFD_URI).expect("WFD_URI is a valid URL")
}

fn serialize<B: AsRef<[u8]>>(message: Message<B>) -> Vec<u8> {
    let mut out = Vec::new();
    // Writing into a Vec cannot run out of space, and rtsp-types reports no
    // other failure for a message it just built.
    message
        .write(&mut out)
        .expect("writing a message into a Vec cannot fail");
    out
}

/// A request against `WFD_URI`, with `Content-Type` only when there is a body.
///
/// rtsp-types adds `Content-Length` itself for a non-empty body and never adds
/// `CSeq`, so the caller's counter is the only sequence source.
fn source_request(cseq: u32, method: Method, body: String) -> Vec<u8> {
    let mut builder = Request::builder(method, Version::V1_0)
        .request_uri(wfd_uri())
        .header(headers::CSEQ, cseq.to_string());
    if !body.is_empty() {
        builder = builder.header(headers::CONTENT_TYPE, "text/parameters");
    }
    serialize(Message::from(builder.build(body.into_bytes())))
}

/// M1 — `OPTIONS *`, sent once, `PRE_M1_DELAY` after the sink connects.
pub fn build_m1(cseq: u32) -> Vec<u8> {
    serialize(Message::from(
        Request::builder(Method::Options, Version::V1_0)
            .header(headers::CSEQ, cseq.to_string())
            .header(headers::REQUIRE, WFD_PROFILE)
            .empty(),
    ))
}

/// M3 — ask the sink what it can do.
pub fn build_m3(cseq: u32) -> Vec<u8> {
    source_request(
        cseq,
        Method::GetParameter,
        format_body(&[
            ("wfd_video_formats", None),
            ("wfd_audio_codecs", None),
            ("wfd_client_rtp_ports", None),
            ("wfd_content_protection", None),
        ]),
    )
}

/// M4 — declare what glint will actually send.
///
/// `local` is the accepted socket's own address: the link layer exposes no IP,
/// so the socket is the only source of truth for the presentation URL. The
/// sink's own RTP ports are echoed back unchanged, which is what both
/// references do.
pub fn build_m4(
    cseq: u32,
    chosen: &ChosenFormat,
    audio: Option<&AudioCodecs>,
    local: SocketAddr,
    sink_ports: &ClientRtpPorts,
) -> Vec<u8> {
    let (table, mask) = bit_for_mode(chosen.width, chosen.height, chosen.fps)
        .expect("negotiate only ever chooses a progressive mode from the tables");
    let (cea, vesa, hh) = match table {
        Table::Cea => (mask, 0, 0),
        Table::Vesa => (0, mask, 0),
        Table::Hh => (0, 0, mask),
    };
    let formats = VideoFormats {
        native: 0,
        preferred_display_mode: 0,
        codecs: vec![H264Codec {
            profile: chosen.profile.bit(),
            level: chosen.level,
            cea,
            vesa,
            hh,
            latency: 0,
            min_slice_size: 0,
            slice_enc_params: 0,
            frame_rate_control: 0,
            max_hres: None,
            max_vres: None,
        }],
    };
    let audio = audio
        .map(WfdParam::format)
        .unwrap_or_else(|| "none".to_string());
    let url = format!("rtsp://{local}/wfd1.0/streamid=0 none");
    source_request(
        cseq,
        Method::SetParameter,
        format_body(&[
            ("wfd_video_formats", Some(&formats.format())),
            ("wfd_audio_codecs", Some(&audio)),
            ("wfd_presentation_URL", Some(&url)),
            ("wfd_client_rtp_ports", Some(&sink_ports.format())),
        ]),
    )
}

/// M5 — trigger the sink's SETUP.
pub fn build_m5(cseq: u32) -> Vec<u8> {
    source_request(
        cseq,
        Method::SetParameter,
        format_body(&[("wfd_trigger_method", Some("SETUP"))]),
    )
}

/// M16 — the keep-alive. No `;timeout=` suffix on the session: GND strips it
/// because it "seems to confuse some clients".
pub fn build_m16(cseq: u32, session: &str) -> Vec<u8> {
    serialize(Message::from(
        Request::builder(Method::GetParameter, Version::V1_0)
            .request_uri(wfd_uri())
            .header(headers::CSEQ, cseq.to_string())
            .header(headers::SESSION, session)
            .build(Vec::new()),
    ))
}

/// The characters a session id may contain. `$` is deliberately absent:
/// Live555-derived sinks strip it, so an id carrying one comes back changed
/// and stops matching the session glint minted.
pub const SESSION_ID_CHARSET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._+-";

/// Ten characters. LG sinks cap a session id at 15 counting the terminating
/// NUL, and GND's field-proven value is 10.
const SESSION_ID_LEN: usize = 10;

/// A session id for one RTSP session, derived from `seed` alone.
///
/// Pure so a test can pin it per seed; the driver supplies the entropy by
/// seeding from the wall clock and a per-session counter, which needs no
/// dependency.
pub fn session_id(seed: u64) -> String {
    // splitmix64 rather than xorshift: xorshift64 has a fixed point at zero, so
    // it needs the state forced nonzero, and doing that by OR-ing in the low
    // bit makes every even seed collide with the odd seed above it. splitmix64
    // advances by addition and is a bijection, so distinct seeds stay distinct
    // with no guard at all.
    let mut state = seed;
    (0..SESSION_ID_LEN)
        .map(|_| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut mixed = state;
            mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            mixed ^= mixed >> 31;
            let index = (mixed % SESSION_ID_CHARSET.len() as u64) as usize;
            SESSION_ID_CHARSET[index] as char
        })
        .collect()
}

/// What glint declares it can encode, as its M3 side of the negotiation.
///
/// One codec entry under both profiles, CEA bits 0-7 and nothing else. The
/// top CEA bit claimed is 1080p30, which is the mode GND ships hardcoded and
/// interops with everywhere; `negotiate` still degrades to any smaller mode
/// this table and the sink share.
pub fn our_video_formats() -> VideoFormats {
    VideoFormats {
        native: 0,
        preferred_display_mode: 0,
        codecs: vec![H264Codec {
            profile: H264Profile::ConstrainedBaseline.bit() | H264Profile::ConstrainedHigh.bit(),
            level: 0x10,
            cea: 0x0000_00ff,
            vesa: 0,
            hh: 0,
            latency: 0,
            min_slice_size: 0,
            slice_enc_params: 0,
            frame_rate_control: 0,
            max_hres: None,
            max_vres: None,
        }],
    }
}

/// The audio to declare in M4, or `None` for a video-only cast.
///
/// Only AAC at mode bit 0 — 48 kHz, two channels — is ever matched, and an
/// offer without it becomes a video-only cast rather than a failed handshake.
/// That is GND's shipping behaviour, and the alternative is a black screen on
/// every LPCM-only sink.
pub fn select_audio(sink: &AudioCodecs) -> Option<AudioCodecs> {
    const AAC_48K_STEREO: u32 = 0x0000_0001;
    sink.0
        .iter()
        .find(|codec| codec.format.eq_ignore_ascii_case("AAC") && codec.modes & AAC_48K_STEREO != 0)
        .map(|_| {
            AudioCodecs(vec![AudioCodec {
                format: "AAC".to_string(),
                modes: AAC_48K_STEREO,
                latency: 0,
            }])
        })
}

/// The parameter lines of a `text/parameters` body, in order.
///
/// Tolerant on purpose: the reference parser records that real peers emit
/// CRLF, bare CR, bare LF and mixed forms, and that a line with no colon is a
/// bare parameter name — which is the M3 request format. Unknown names are
/// kept rather than rejected, because vendor extensions (`microsoft_*`,
/// `intel_*`) are normal in an M3 reply and never an error.
pub fn parse_body(bytes: &[u8]) -> Vec<(String, Option<String>)> {
    String::from_utf8_lossy(bytes)
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| match line.split_once(':') {
            Some((name, value)) => (name.trim().to_string(), Some(value.trim().to_string())),
            None => (line.to_string(), None),
        })
        .collect()
}

/// A parameter's value, by ASCII-case-insensitive name.
pub fn body_value<'a>(body: &'a [(String, Option<String>)], name: &str) -> Option<&'a str> {
    body.iter()
        .find(|(found, _)| found.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_deref())
}

/// Whether a parameter is present at all — a bare name has no value but is
/// still there, which is how the M3 request and M13 both read.
pub fn body_has(body: &[(String, Option<String>)], name: &str) -> bool {
    body.iter()
        .any(|(found, _)| found.eq_ignore_ascii_case(name))
}

/// The inverse of `parse_body`, for the bodies glint sends.
pub fn format_body(params: &[(&str, Option<&str>)]) -> String {
    params
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{name}: {value}\r\n"),
            None => format!("{name}\r\n"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_body_splits_on_the_first_colon_only() {
        // A presentation URL's value contains colons of its own, so splitting
        // on every colon would truncate it.
        // act
        let parsed = parse_body(b"wfd_presentation_URL: rtsp://1.2.3.4:7236/wfd1.0 none\r\n");
        // assert
        assert_eq!(
            parsed,
            vec![(
                "wfd_presentation_URL".to_string(),
                Some("rtsp://1.2.3.4:7236/wfd1.0 none".to_string())
            )]
        );
    }

    #[test]
    fn parse_body_reads_a_bare_name_as_a_name_with_no_value() {
        // That is exactly the M3 request body format.
        // act
        let parsed = parse_body(b"wfd_video_formats\r\nwfd_audio_codecs\r\n");
        // assert
        assert_eq!(
            parsed,
            vec![
                ("wfd_video_formats".to_string(), None),
                ("wfd_audio_codecs".to_string(), None),
            ]
        );
    }

    #[test]
    fn parse_body_tolerates_every_line_ending_form_and_blank_lines() {
        // rtsp-types refuses bare-LF in the HEADERS; bodies are ours to read,
        // and the reference parser records CRLF, bare CR, bare LF and mixed
        // forms all occurring in the wild.
        // act
        let parsed = parse_body(b"a: 1\n\r\nb: 2\r\n\n");
        // assert
        assert_eq!(
            parsed,
            vec![
                ("a".to_string(), Some("1".to_string())),
                ("b".to_string(), Some("2".to_string())),
            ]
        );
    }

    #[test]
    fn parse_body_trims_the_whitespace_around_a_name_and_a_value() {
        // The reference parser tolerates `key : value` spacing, so a value must
        // not arrive carrying the separator's whitespace.
        // act
        let parsed = parse_body(b"  wfd_audio_codecs :   AAC 00000001 00   \r\n");
        // assert
        assert_eq!(
            parsed,
            vec![(
                "wfd_audio_codecs".to_string(),
                Some("AAC 00000001 00".to_string())
            )]
        );
    }

    #[test]
    fn a_body_value_is_found_regardless_of_case() {
        // The reference sink matches parameter names case-insensitively, and a
        // vendor may echo a name back in another case.
        // arrange
        let parsed = parse_body(b"WFD_Video_Formats: 40 00\r\n");
        // act & assert
        assert_eq!(body_value(&parsed, "wfd_video_formats"), Some("40 00"));
        assert_eq!(body_value(&parsed, "wfd_audio_codecs"), None);
    }

    #[test]
    fn a_bare_name_is_present_even_though_it_has_no_value() {
        // arrange
        let parsed = parse_body(b"wfd_idr_request\r\n");
        // act & assert
        assert!(body_has(&parsed, "wfd_idr_request"));
        assert_eq!(body_value(&parsed, "wfd_idr_request"), None);
    }

    #[test]
    fn format_body_joins_with_crlf_and_ends_with_one() {
        // act
        let formatted = format_body(&[
            ("wfd_video_formats", None),
            ("wfd_audio_codecs", Some("none")),
        ]);
        // assert
        assert_eq!(formatted, "wfd_video_formats\r\nwfd_audio_codecs: none\r\n");
    }

    #[test]
    fn format_body_of_nothing_is_empty_not_a_bare_crlf() {
        // M16's body must be genuinely empty: that emptiness is what makes
        // rtsp-types omit Content-Length, and it is what the reference
        // classifier reads as M16 rather than M3.
        // act & assert
        assert_eq!(format_body(&[]), "");
    }

    #[test]
    fn a_formatted_body_parses_back_to_what_it_was_built_from() {
        // arrange
        let params = [
            ("wfd_video_formats", Some("00 00 02 10 00000080")),
            ("wfd_audio_codecs", Some("none")),
            ("wfd_trigger_method", None),
        ];
        // act
        let round_tripped = parse_body(format_body(&params).as_bytes());
        // assert
        assert_eq!(
            round_tripped,
            params
                .iter()
                .map(|(name, value)| (name.to_string(), value.map(str::to_string)))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_session_id_is_ten_characters_long() {
        // Ten, not fifteen: LG's limit is 15 counting the terminating NUL in
        // GND's own counting, and GND's field value is 10.
        for seed in 0..1_000u64 {
            // act & assert
            assert_eq!(session_id(seed).chars().count(), 10, "seed {seed}");
        }
    }

    #[test]
    fn the_session_id_charset_excludes_the_dollar_sign() {
        // Live555-derived sinks strip `$` out of a session id, so an id
        // carrying one comes back changed and stops matching what we minted.
        // act & assert
        assert!(
            !SESSION_ID_CHARSET.contains(&b'$'),
            "the charset admits `$`"
        );
    }

    #[test]
    fn a_session_id_uses_only_the_permitted_characters() {
        // Asserted against a literal predicate rather than against the charset
        // constant, so widening that constant cannot make this test agree
        // with it.
        for seed in 0..1_000u64 {
            for byte in session_id(seed).bytes() {
                // assert
                assert!(
                    byte.is_ascii_alphanumeric() || b"._+-".contains(&byte),
                    "seed {seed} produced {:?}",
                    byte as char
                );
            }
        }
    }

    #[test]
    fn a_session_id_is_deterministic_per_seed() {
        // act & assert
        assert_eq!(session_id(42), session_id(42));
    }

    #[test]
    fn different_seeds_give_different_session_ids() {
        // arrange & act
        let ids: std::collections::BTreeSet<String> = (0..500u64).map(session_id).collect();
        // assert
        assert_eq!(ids.len(), 500, "seeds collided");
    }

    #[test]
    fn our_video_formats_offers_one_codec_entry_under_both_profiles() {
        // act
        let ours = our_video_formats();
        // assert
        assert_eq!(ours.native, 0);
        assert_eq!(ours.preferred_display_mode, 0);
        assert_eq!(ours.codecs.len(), 1);
        assert_eq!(ours.codecs[0].profile, 0x03);
        assert_eq!(ours.codecs[0].level, 0x10);
    }

    #[test]
    fn our_video_formats_claims_cea_modes_only() {
        // The VESA and Handheld tables are display and phone timings; a
        // desktop cast has no business claiming them, and every one of them
        // would need its own encoder caps clause.
        // act
        let codec = &our_video_formats().codecs[0];
        // assert
        assert_eq!(codec.cea, 0x0000_00ff);
        assert_eq!((codec.vesa, codec.hh), (0, 0));
    }

    #[test]
    fn the_highest_cea_mode_we_claim_is_1080p30() {
        // Capping the claim at 1080p30 is GND's field lesson: it ships that
        // mode hardcoded and interops everywhere. `negotiate` still degrades
        // to any smaller mode the sink and this table share.
        // arrange
        let cea = our_video_formats().codecs[0].cea;
        // act
        let top = (u32::BITS - 1 - cea.leading_zeros()) as usize;
        let mode = crate::wfd::modes::CEA_MODES[top];
        // assert
        assert_eq!((mode.width, mode.height, mode.fps), (1920, 1080, 30));
        assert!(!mode.interlaced);
    }

    #[test]
    fn we_declare_no_maximum_resolution_override() {
        // `negotiate` deliberately ignores max_hres/max_vres, so declaring
        // them here would be a claim nothing on either side reads.
        // act
        let codec = &our_video_formats().codecs[0];
        // assert
        assert_eq!((codec.max_hres, codec.max_vres), (None, None));
    }

    #[test]
    fn select_audio_takes_aac_at_mode_bit_zero() {
        // 48 kHz two-channel AAC-LC: the only audio mode GND ever matches.
        // arrange: the reference sink's own default value
        let sink = AudioCodecs::parse("AAC 00000007 00").unwrap();
        // act
        let chosen = select_audio(&sink).expect("AAC bit 0 is on offer");
        // assert
        assert_eq!(chosen.format(), "AAC 00000001 00");
    }

    #[test]
    fn select_audio_falls_back_to_video_only_for_an_lpcm_only_sink() {
        // GND's shipping behaviour: the handshake does NOT fail, the cast just
        // carries no audio. A hard failure here would black-screen every
        // LPCM-only sink.
        // arrange
        let sink = AudioCodecs::parse("LPCM 00000002 00").unwrap();
        // act & assert
        assert_eq!(select_audio(&sink), None);
    }

    #[test]
    fn select_audio_refuses_aac_that_does_not_offer_mode_bit_zero() {
        // arrange: AAC, but only mode bit 1
        let sink = AudioCodecs::parse("AAC 00000002 00").unwrap();
        // act & assert
        assert_eq!(select_audio(&sink), None);
    }

    #[test]
    fn select_audio_of_an_empty_offer_is_video_only() {
        // arrange
        let sink = AudioCodecs::parse("none").unwrap();
        // act & assert
        assert_eq!(select_audio(&sink), None);
    }

    #[test]
    fn select_audio_finds_aac_behind_other_codecs_and_ignores_their_names() {
        // A sink may list LPCM and AC3 first; the reply must still be the one
        // AAC mode glint can actually encode, not an echo of the offer.
        // arrange
        let sink =
            AudioCodecs::parse("LPCM 00000002 00, AC3 00000001 00, AAC 00000003 04").unwrap();
        // act
        let chosen = select_audio(&sink).expect("AAC bit 0 is on offer");
        // assert
        assert_eq!(chosen.format(), "AAC 00000001 00");
    }

    /// The negotiated format the M4 tests build on: 1080p30 under Constrained
    /// High at the reference sink's level 4.2.
    fn chosen_1080p30() -> ChosenFormat {
        ChosenFormat {
            width: 1920,
            height: 1080,
            fps: 30,
            profile: H264Profile::ConstrainedHigh,
            level: 0x10,
        }
    }

    fn sink_ports() -> ClientRtpPorts {
        ClientRtpPorts::parse("RTP/AVP/UDP;unicast 19000 0 mode=play").unwrap()
    }

    fn local() -> SocketAddr {
        "192.168.1.9:7236".parse().unwrap()
    }

    fn utf8(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).expect("glint only ever builds UTF-8 messages")
    }

    #[test]
    fn m1_is_options_star_requiring_the_wfd_profile() {
        // No request URI at all: `OPTIONS *` is the form both references send,
        // and the reply's Public is logged rather than validated because
        // source-impl's strict three-token check kills a spec-legal reply.
        // act
        let built = build_m1(1);
        // assert
        assert_eq!(
            utf8(built),
            "OPTIONS * RTSP/1.0\r\n\
CSeq: 1\r\n\
Require: org.wfa.wfd1.0\r\n\
\r\n"
        );
    }

    #[test]
    fn m3_asks_for_exactly_four_parameters_by_bare_name() {
        // wfd_content_protection is glint's addition to GND's list: negotiate
        // already consumes it, and an honest HdcpRequired failure beats a
        // black screen.
        // act
        let built = build_m3(2);
        // assert
        assert_eq!(
            utf8(built),
            "GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
Content-Length: 83\r\n\
Content-Type: text/parameters\r\n\
CSeq: 2\r\n\
\r\n\
wfd_video_formats\r\n\
wfd_audio_codecs\r\n\
wfd_client_rtp_ports\r\n\
wfd_content_protection\r\n"
        );
    }

    #[test]
    fn m4_carries_the_whole_negotiated_parameter_set() {
        // One bit set across the three masks — CEA bit 7 is 00000080 — the
        // sink's own level echoed back, the presentation URL taken from the
        // accepted socket, and the sink's own RTP ports echoed.
        // arrange
        let audio = select_audio(&AudioCodecs::parse("AAC 00000007 00").unwrap()).unwrap();
        // act
        let built = build_m4(3, &chosen_1080p30(), Some(&audio), local(), &sink_ports());
        // assert
        assert_eq!(
            utf8(built),
            "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
Content-Length: 251\r\n\
Content-Type: text/parameters\r\n\
CSeq: 3\r\n\
\r\n\
wfd_video_formats: 00 00 02 10 00000080 00000000 00000000 00 0000 0000 00 none none\r\n\
wfd_audio_codecs: AAC 00000001 00\r\n\
wfd_presentation_URL: rtsp://192.168.1.9:7236/wfd1.0/streamid=0 none\r\n\
wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n"
        );
    }

    #[test]
    fn m4_declares_no_audio_as_the_literal_none() {
        // act
        let built = build_m4(3, &chosen_1080p30(), None, local(), &sink_ports());
        // assert
        assert!(utf8(built).contains("wfd_audio_codecs: none\r\n"));
    }

    #[test]
    fn m4_sets_exactly_one_bit_across_the_three_video_masks() {
        // The reference sink reads the LOWEST set bit, so a second bit
        // anywhere makes it decode a mode glint never encoded.
        // act
        let built = utf8(build_m4(3, &chosen_1080p30(), None, local(), &sink_ports()));
        // arrange: fields 4, 5 and 6 of the value are the cea/vesa/hh masks
        let line = built
            .lines()
            .find(|line| line.starts_with("wfd_video_formats:"))
            .expect("M4 carries wfd_video_formats");
        let value = line
            .split_once(": ")
            .expect("the parameter line has a value")
            .1;
        let fields: Vec<&str> = value.split_whitespace().collect();
        // act
        let bits: u32 = fields[4..7]
            .iter()
            .map(|field| u32::from_str_radix(field, 16).unwrap().count_ones())
            .sum();
        // assert
        assert_eq!(bits, 1, "got: {line}");
    }

    #[test]
    fn m4_puts_a_handheld_mode_in_the_handheld_mask() {
        // The bit belongs in the table it came from; putting every mode in the
        // CEA mask would name a completely different resolution.
        // arrange: 960x540p60 is HH bit 9
        let chosen = ChosenFormat {
            width: 960,
            height: 540,
            fps: 60,
            profile: H264Profile::ConstrainedBaseline,
            level: 0x08,
        };
        // act
        let built = utf8(build_m4(3, &chosen, None, local(), &sink_ports()));
        // assert
        assert!(
            built.contains("wfd_video_formats: 00 00 01 08 00000000 00000000 00000200 "),
            "got: {built}"
        );
    }

    #[test]
    fn m5_triggers_setup_and_nothing_else() {
        // The only trigger glint ever sends; the reference sink implements no
        // other and silently ignores the rest.
        // act
        let built = build_m5(4);
        // assert
        assert_eq!(
            utf8(built),
            "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
Content-Length: 27\r\n\
Content-Type: text/parameters\r\n\
CSeq: 4\r\n\
\r\n\
wfd_trigger_method: SETUP\r\n"
        );
    }

    #[test]
    fn m16_is_a_body_less_get_parameter_carrying_the_session() {
        // The EMPTY body is what makes this M16 rather than M3 to the
        // reference classifier, and an empty body is why no Content-Length is
        // emitted — which the sink's own replies also omit.
        // act
        let built = build_m16(5, "abcdefghij");
        // assert
        assert_eq!(
            utf8(built),
            "GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
CSeq: 5\r\n\
Session: abcdefghij\r\n\
\r\n"
        );
    }

    #[test]
    fn no_source_message_carries_a_session_timeout_suffix() {
        // GND strips ";timeout=30" from outgoing Session headers because it
        // "seems to confuse some clients".
        // act & assert
        assert!(!utf8(build_m16(5, "abcdefghij")).contains("timeout="));
    }
}
