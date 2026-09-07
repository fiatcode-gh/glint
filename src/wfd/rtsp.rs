//! The RTSP transport: the port, the frame decoder, the listener and the
//! driver that connects a socket to `wfd::flow`.
//!
//! Everything that decides anything lives in `flow`; this module moves bytes
//! and supplies elapsed time. The split is what lets the whole M1-M8 exchange
//! be tested without a socket, and the socket path be tested without a
//! television.

use rtsp_types::{Message, ParseError};

/// TCP port of the WFD source's RTSP listener. Wi-Fi Display fixes it at 7236,
/// and the value is also advertised inside `WFD_SOURCE_IES` (bytes 5-6, big
/// endian) — the two must agree or sinks dial a port nobody answers.
pub const RTSP_PORT: u16 = 7236;

/// A byte stream the decoder cannot read as RTSP.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the peer sent bytes that are not RTSP: {escaped}")]
pub struct FrameError {
    /// The offending line, escaped so a control byte in it cannot corrupt the
    /// log that exists to diagnose it.
    pub escaped: String,
}

/// Bytes in, RTSP messages out.
///
/// Pure, holding only the bytes not yet consumed, so a test can feed it one
/// byte at a time and the driver can hand it whatever a read happened to
/// return. rtsp-types reports how many bytes each message consumed, and
/// draining exactly that many is what keeps a pipelined pair intact.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next complete message, or `None` while more bytes are needed.
    ///
    /// `Incomplete` is not an error: a short read is the normal case. Only a
    /// genuinely malformed stream is, and it carries the bytes for the log.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Message<Vec<u8>>>, FrameError> {
        match Message::parse(&self.buffer) {
            Ok((message, consumed)) => {
                self.buffer.drain(..consumed);
                Ok(Some(message))
            }
            Err(ParseError::Incomplete(_)) => Ok(None),
            Err(ParseError::Error) => Err(FrameError {
                escaped: self
                    .buffer
                    .split(|byte| *byte == b'\n')
                    .next()
                    .map(|line| String::from_utf8_lossy(line).escape_debug().to_string())
                    .unwrap_or_default(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::network_manager::WFD_SOURCE_IES;

    #[test]
    fn the_advertised_control_port_is_the_port_we_listen_on() {
        // The information elements tell a sink where to dial; the listener
        // decides who answers. The two must agree or sinks dial a port nobody
        // is on. This is the first code tying the link layer's advertisement
        // to the transport's behaviour.
        // act
        let advertised = u16::from_be_bytes([WFD_SOURCE_IES[5], WFD_SOURCE_IES[6]]);
        // assert
        assert_eq!(advertised, RTSP_PORT);
    }

    #[test]
    fn the_decoder_yields_two_back_to_back_messages_then_nothing() {
        // arrange
        let mut decoder = FrameDecoder::default();
        decoder.push(b"OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\nOPTIONS * RTSP/1.0\r\nCSeq: 2\r\n\r\n");
        // act & assert
        assert!(decoder.next().unwrap().is_some());
        assert!(decoder.next().unwrap().is_some());
        assert!(decoder.next().unwrap().is_none());
    }

    #[test]
    fn the_decoder_reassembles_a_message_fed_one_byte_at_a_time() {
        // A TCP read boundary can fall anywhere, so the buffer must never lose
        // a byte across a partial read.
        // arrange
        let raw = b"GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 3\r\n\
Content-Type: text/parameters\r\nContent-Length: 19\r\n\r\nwfd_video_formats\r\n";
        let mut decoder = FrameDecoder::default();
        let mut yielded = Vec::new();
        // act
        for byte in raw {
            decoder.push(&[*byte]);
            while let Some(message) = decoder.next().unwrap() {
                yielded.push(message);
            }
        }
        // assert
        assert_eq!(yielded.len(), 1);
        let Message::Request(request) = &yielded[0] else {
            panic!("expected a request, got {:?}", yielded[0]);
        };
        assert_eq!(request.body(), b"wfd_video_formats\r\n");
    }

    #[test]
    fn the_decoder_keeps_a_trailing_partial_message_for_the_next_read() {
        // arrange: one whole message plus the first bytes of another
        let mut decoder = FrameDecoder::default();
        decoder.push(b"OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\nOPTIONS * RTS");
        // act
        assert!(decoder.next().unwrap().is_some());
        assert!(decoder.next().unwrap().is_none());
        decoder.push(b"P/1.0\r\nCSeq: 2\r\n\r\n");
        // assert
        assert!(decoder.next().unwrap().is_some());
    }

    #[test]
    fn an_incomplete_message_is_not_an_error() {
        // arrange
        let mut decoder = FrameDecoder::default();
        decoder.push(b"OPTIONS * RTSP/1.0\r\nCSe");
        // act & assert
        assert!(decoder.next().unwrap().is_none());
    }

    #[test]
    fn an_empty_buffer_is_not_an_error() {
        // arrange
        let mut decoder = FrameDecoder::default();
        // act & assert
        assert!(decoder.next().unwrap().is_none());
    }

    #[test]
    fn the_decoder_reports_a_malformed_line_with_its_bytes_escaped() {
        // The escaped dump is the only diagnostic instrument for a television
        // whose real byte stream nobody has seen yet, and escaping is what
        // stops a control byte corrupting the log meant to diagnose it.
        // arrange
        let mut decoder = FrameDecoder::default();
        decoder.push(b"NOT RTSP AT ALL\x01\r\n");
        // act
        let error = decoder.next().expect_err("a malformed line is an error");
        // assert
        assert!(error.escaped.contains("NOT RTSP AT ALL"), "got: {error}");
        assert!(error.escaped.contains("\\u{1}"), "got: {error}");
    }

    #[test]
    fn the_decoder_reports_only_the_first_line_of_a_malformed_stream() {
        // A television that floods the socket must not put its whole buffer
        // into one log line.
        // arrange
        let mut decoder = FrameDecoder::default();
        decoder.push(b"garbage one\r\ngarbage two\r\n");
        // act
        let error = decoder.next().expect_err("a malformed line is an error");
        // assert
        assert!(error.escaped.contains("garbage one"), "got: {error}");
        assert!(!error.escaped.contains("garbage two"), "got: {error}");
    }

    #[test]
    fn the_decoder_surfaces_an_interleaved_data_frame() {
        // WFD does not use them, so the caller logs and drops it — but
        // dropping it inside the decoder would hide a misbehaving sink.
        // arrange
        let mut decoder = FrameDecoder::default();
        decoder.push(&[b'$', 0x00, 0x00, 0x02, 0xaa, 0xbb]);
        // act
        let message = decoder.next().unwrap().expect("a data frame is a message");
        // assert
        assert!(matches!(message, Message::Data(_)));
    }
}
