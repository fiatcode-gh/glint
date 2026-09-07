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
}
