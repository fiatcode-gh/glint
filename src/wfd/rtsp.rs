//! The RTSP transport: the port, the frame decoder, the listener and the
//! driver that connects a socket to `wfd::flow`.
//!
//! Everything that decides anything lives in `flow`; this module moves bytes
//! and supplies elapsed time. The split is what lets the whole M1-M8 exchange
//! be tested without a socket, and the socket path be tested without a
//! television.

use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rtsp_types::{Message, ParseError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::wfd::flow::{Flow, FlowEvent, Outbound};

/// How often the driver asks the flow whether a timer has come due. Well under
/// every deadline the flow holds; only the bound matters, not the number.
const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// One read's worth of buffer. The decoder reassembles whatever a read
/// actually returns, so this only trades syscalls against memory.
const READ_CHUNK: usize = 4096;

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

#[derive(Debug, thiserror::Error)]
pub enum RtspError {
    #[error("the RTSP socket failed: {0}")]
    Io(#[from] std::io::Error),
}

/// The source's RTSP listener.
pub struct RtspListener {
    inner: TcpListener,
}

impl RtspListener {
    /// Tests bind an ephemeral localhost address; the daemon and the examples
    /// bind `0.0.0.0:RTSP_PORT`.
    pub async fn bind(address: SocketAddr) -> std::io::Result<Self> {
        Ok(RtspListener {
            inner: TcpListener::bind(address).await?,
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Accept sinks and drive one flow at a time, reporting what happens on
    /// `events`.
    ///
    /// A second connection arriving while a flow is live is closed at once.
    /// That is a deliberate deviation from GND, which gives an extra
    /// connection its own full handshake but tracks only the first, leaving it
    /// orphaned in a half-alive state.
    pub async fn serve(&self, events: mpsc::Sender<FlowEvent>) -> Result<(), RtspError> {
        let mut sessions = 0u64;
        loop {
            let (stream, peer) = self.inner.accept().await?;
            sessions += 1;
            let local = stream.local_addr()?;
            tracing::info!(%peer, "a sink connected");

            let outcome = tokio::select! {
                served = drive(stream, local, peer, session_seed(sessions), &events) => served,
                turned_away = self.turn_away_extras() => turned_away,
            };
            if let Err(error) = outcome {
                tracing::warn!(%peer, %error, "the RTSP session ended in an error");
            }
        }
    }

    /// Close every further connection for as long as one sink is being served.
    /// Only ever returns by failing, so it never wins the race against the
    /// session it is protecting.
    async fn turn_away_extras(&self) -> Result<(), RtspError> {
        loop {
            let (extra, peer) = self.inner.accept().await?;
            tracing::warn!(
                %peer,
                "a second sink connected while one is already being served; closing it"
            );
            drop(extra);
        }
    }
}

/// A seed no two sessions share.
///
/// The wall clock gives a fresh value per run and the counter separates
/// sessions within one run. A clock before the epoch costs only the id's
/// unpredictability — the counter still keeps two sessions apart — so it falls
/// back rather than failing a cast over it.
fn session_seed(counter: u64) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ counter
}

/// Run one sink's flow to its end.
///
/// The only place elapsed time enters the protocol: `Instant` is read here and
/// handed to the flow as a plain `Duration`, which is what keeps every
/// deadline testable without a clock.
async fn drive(
    mut stream: TcpStream,
    local: SocketAddr,
    peer: SocketAddr,
    seed: u64,
    events: &mpsc::Sender<FlowEvent>,
) -> Result<(), RtspError> {
    let started = Instant::now();
    let mut flow = Flow::new(local, peer, seed);
    let mut decoder = FrameDecoder::default();
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut ticker = tokio::time::interval(TICK_INTERVAL);

    loop {
        let (out, produced) = tokio::select! {
            read = stream.read(&mut buffer) => match read? {
                0 => {
                    tracing::info!(%peer, "the sink hung up");
                    emit(events, flow.on_hangup()).await;
                    return Ok(());
                }
                count => {
                    decoder.push(&buffer[..count]);
                    consume(&mut decoder, &mut flow, started.elapsed())
                }
            },
            _ = ticker.tick() => flow.on_tick(started.elapsed()),
        };

        // Bytes before events: a 200 to TEARDOWN has to reach the sink before
        // anything acts on the teardown and closes the socket.
        for item in &out {
            if let Outbound::Bytes(bytes) = item {
                stream.write_all(bytes).await?;
            }
        }
        if !emit(events, produced).await {
            return Ok(());
        }
        if out.iter().any(|item| matches!(item, Outbound::Close)) {
            let _ = stream.shutdown().await;
            return Ok(());
        }
    }
}

/// Feed the flow every whole message the buffer now holds.
///
/// A stream glint cannot parse fails the flow rather than being skipped: the
/// message already arrived, so waiting for it again would wait forever, and a
/// television whose bytes rtsp-types rejects is exactly what the escaped log
/// exists to diagnose.
fn consume(
    decoder: &mut FrameDecoder,
    flow: &mut Flow,
    now: Duration,
) -> (Vec<Outbound>, Vec<FlowEvent>) {
    let mut out = Vec::new();
    let mut produced = Vec::new();
    loop {
        match decoder.next() {
            Ok(Some(message)) => {
                let (more_out, more_events) = flow.on_message(&message, now);
                out.extend(more_out);
                produced.extend(more_events);
            }
            Ok(None) => return (out, produced),
            Err(error) => {
                tracing::warn!(%error, "the sink sent bytes glint cannot read as RTSP");
                let (more_out, more_events) = flow.abort(error.to_string());
                out.extend(more_out);
                produced.extend(more_events);
                return (out, produced);
            }
        }
    }
}

/// Hand the events to whoever is listening. `false` means nobody is any more,
/// which makes the rest of the cast pointless.
async fn emit(events: &mpsc::Sender<FlowEvent>, produced: Vec<FlowEvent>) -> bool {
    for event in produced {
        if events.send(event).await.is_err() {
            tracing::debug!("nothing is listening for flow events; ending the session");
            return false;
        }
    }
    true
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
