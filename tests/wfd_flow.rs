//! The M1-M8 exchange over a real localhost socket, against a scripted sink.
//!
//! The sink is scripted from the miraclecast-family sink's documented
//! behaviour: it answers M1, sends M2, answers M3 with its default parameter
//! values, answers M4 and M5, then sends SETUP and PLAY and sits.
//!
//! Deliberately asserts NO timing. Every deadline is pinned as pure
//! arithmetic in `wfd::flow`'s own tests, where there is no clock and no
//! socket to make it flaky; here the point is that the bytes and the events
//! are right. So this suite waits for M1 rather than clocking it, which is
//! also why changing `PRE_M1_DELAY` cannot fail anything here.

use std::net::SocketAddr;
use std::time::Duration;

use glint::wfd::flow::FlowEvent;
use glint::wfd::rtsp::{FrameDecoder, RtspListener};
use rtsp_types::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// Long enough that a loaded machine does not fail the test, short enough that
/// a genuinely wedged flow fails instead of hanging the suite. Only the
/// bound's existence is a contract; the number is not.
const PATIENCE: Duration = Duration::from_secs(5);

/// The reference sink's documented default M3 reply values.
const SINK_CAPABILITIES: &str = "wfd_video_formats: 00 00 03 10 0001ffff 1fffffff 00001fff 00 0000 0000 00 none none\r\n\
wfd_audio_codecs: AAC 00000007 00\r\n\
wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n\
wfd_content_protection: none\r\n";

/// One scripted sink on the far end of the source's socket.
struct Sink {
    stream: TcpStream,
    decoder: FrameDecoder,
}

impl Sink {
    async fn connect(to: SocketAddr) -> Sink {
        Sink {
            stream: TcpStream::connect(to).await.expect("the listener is up"),
            decoder: FrameDecoder::default(),
        }
    }

    /// The next message the source sent, and its exact bytes.
    async fn next_message(&mut self) -> (Message<Vec<u8>>, String) {
        let mut buffer = [0u8; 4096];
        loop {
            if let Some(message) = self.decoder.next().expect("the source speaks valid RTSP") {
                let mut bytes = Vec::new();
                message
                    .write(&mut bytes)
                    .expect("writing to a Vec succeeds");
                return (message, String::from_utf8(bytes).expect("UTF-8"));
            }
            let read = tokio::time::timeout(PATIENCE, self.stream.read(&mut buffer))
                .await
                .expect("the source sent something before the deadline")
                .expect("the socket is readable");
            assert_ne!(read, 0, "the source hung up mid-exchange");
            self.decoder.push(&buffer[..read]);
        }
    }

    async fn send(&mut self, raw: &str) {
        self.stream
            .write_all(raw.as_bytes())
            .await
            .expect("the socket is writable");
    }

    /// A 200 reply with a correct Content-Length, since a body without one is
    /// silently ignored by the parser on the other side.
    async fn reply(&mut self, cseq: u32, body: &str) {
        if body.is_empty() {
            self.send(&format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n\r\n"))
                .await;
        } else {
            self.send(&format!(
                "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: text/parameters\r\n\
Content-Length: {}\r\n\r\n{body}",
                body.len()
            ))
            .await;
        }
    }
}

/// The CSeq of a request the source sent.
fn cseq_of(text: &str) -> u32 {
    text.lines()
        .find_map(|line| line.strip_prefix("CSeq: "))
        .expect("every source message carries a CSeq")
        .trim()
        .parse()
        .expect("the CSeq is a number")
}

/// A listener on an ephemeral localhost port, serving in a background task.
async fn source() -> (SocketAddr, mpsc::Receiver<FlowEvent>) {
    let listener = RtspListener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("an ephemeral localhost port is available");
    let address = listener.local_addr().expect("the socket is bound");
    let (sender, receiver) = mpsc::channel(32);
    tokio::spawn(async move {
        let _ = listener.serve(sender).await;
    });
    (address, receiver)
}

async fn next_event(events: &mut mpsc::Receiver<FlowEvent>) -> FlowEvent {
    tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("an event arrived before the deadline")
        .expect("the flow is still running")
}

/// Walk the sink from connect to PLAY, returning every source message's bytes
/// in order, plus the source's own address.
async fn walk_to_play(sink: &mut Sink, local: SocketAddr, capabilities: &str) -> Vec<String> {
    let mut sent = Vec::new();

    // M1
    let (_, m1) = sink.next_message().await;
    sent.push(m1.clone());
    sink.reply(cseq_of(&m1), "").await;

    // M2 — the sink sends its own OPTIONS the moment it has answered M1, and
    // the source must answer it whatever else it is doing.
    sink.send("OPTIONS * RTSP/1.0\r\nCSeq: 900\r\nRequire: org.wfa.wfd1.0\r\n\r\n")
        .await;

    // The next two messages are the M2 reply and M3, in whichever order the
    // source got to them.
    let (_, first) = sink.next_message().await;
    let (_, second) = sink.next_message().await;
    let (m2_reply, m3) = if first.starts_with("RTSP/1.0") {
        (first, second)
    } else {
        (second, first)
    };
    sent.push(m2_reply);
    sent.push(m3.clone());
    sink.reply(cseq_of(&m3), capabilities).await;

    // M4
    let (_, m4) = sink.next_message().await;
    sent.push(m4.clone());
    sink.reply(cseq_of(&m4), "").await;

    // M5
    let (_, m5) = sink.next_message().await;
    sent.push(m5.clone());
    sink.reply(cseq_of(&m5), "").await;

    // M6 — the sink sets up, naming the port it will actually listen on
    sink.send(&format!(
        "SETUP rtsp://{local}/wfd1.0/streamid=0 RTSP/1.0\r\n\
CSeq: 901\r\nTransport: RTP/AVP/UDP;unicast;client_port=19000\r\n\r\n"
    ))
    .await;
    let (_, setup_reply) = sink.next_message().await;
    sent.push(setup_reply);

    // M7
    sink.send(&format!(
        "PLAY rtsp://{local}/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 902\r\nSession: x\r\n\r\n"
    ))
    .await;
    let (_, play_reply) = sink.next_message().await;
    sent.push(play_reply);

    sent
}

#[tokio::test]
async fn the_full_exchange_is_byte_exact_and_names_the_rtp_destination() {
    // arrange
    let (local, mut events) = source().await;
    let mut sink = Sink::connect(local).await;

    // act
    let sent = walk_to_play(&mut sink, local, SINK_CAPABILITIES).await;

    // assert: M1 — OPTIONS *, requiring the WFD profile, CSeq 1
    assert_eq!(
        sent[0],
        "OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nRequire: org.wfa.wfd1.0\r\n\r\n"
    );

    // the M2 reply — the profile token first in Public
    assert_eq!(
        sent[1],
        "RTSP/1.0 200 OK\r\nCSeq: 900\r\n\
Public: org.wfa.wfd1.0, OPTIONS, GET_PARAMETER, SET_PARAMETER, SETUP, PLAY, PAUSE, TEARDOWN\r\n\r\n"
    );

    // M3 — four bare names against the literal localhost URI
    assert_eq!(
        sent[2],
        "GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
Content-Length: 83\r\nContent-Type: text/parameters\r\nCSeq: 2\r\n\r\n\
wfd_video_formats\r\nwfd_audio_codecs\r\nwfd_client_rtp_ports\r\nwfd_content_protection\r\n"
    );

    // M4 — 1080p30 as CEA bit 7 alone, the sink's level echoed, AAC at bit 0,
    // the presentation URL taken from this very socket, the sink's ports back
    let body = format!(
        "wfd_video_formats: 00 00 02 10 00000080 00000000 00000000 00 0000 0000 00 none none\r\n\
wfd_audio_codecs: AAC 00000001 00\r\n\
wfd_presentation_URL: rtsp://{local}/wfd1.0/streamid=0 none\r\n\
wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n"
    );
    assert_eq!(
        sent[3],
        format!(
            "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
Content-Length: {}\r\nContent-Type: text/parameters\r\nCSeq: 3\r\n\r\n{body}",
            body.len()
        )
    );

    // M5 — the SETUP trigger and nothing else
    assert_eq!(
        sent[4],
        "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n\
Content-Length: 27\r\nContent-Type: text/parameters\r\nCSeq: 4\r\n\r\n\
wfd_trigger_method: SETUP\r\n"
    );

    // the M6 reply — a 10-character session and our own server ports
    let session = sent[5]
        .lines()
        .find_map(|line| line.strip_prefix("Session: "))
        .expect("the SETUP reply mints a session")
        .to_string();
    assert_eq!(session.len(), 10, "session id: {session}");
    assert_eq!(
        sent[5],
        format!(
            "RTSP/1.0 200 OK\r\nCSeq: 901\r\nSession: {session}\r\n\
Transport: RTP/AVP/UDP;unicast;client_port=19000;server_port=16384-16385\r\n\r\n"
        )
    );

    // the M7 reply — the same session, no timeout suffix
    assert_eq!(
        sent[6],
        format!("RTSP/1.0 200 OK\r\nCSeq: 902\r\nSession: {session}\r\n\r\n")
    );

    // the events: the raw capture first, then the destination
    let captured = next_event(&mut events).await;
    assert_eq!(
        captured,
        FlowEvent::M3Captured(SINK_CAPABILITIES.as_bytes().to_vec())
    );
    assert_eq!(
        next_event(&mut events).await,
        FlowEvent::Negotiated {
            width: 1920,
            height: 1080,
            fps: 30,
            audio: true,
        }
    );
    assert_eq!(
        next_event(&mut events).await,
        FlowEvent::Play {
            rtp_host: "127.0.0.1".to_string(),
            rtp_port: 19000,
        }
    );
}

#[tokio::test]
async fn a_sink_hangup_after_play_is_a_teardown() {
    // The reference sink never sends M8 — a TCP hangup is how a cast ends in
    // the field.
    // arrange
    let (local, mut events) = source().await;
    let mut sink = Sink::connect(local).await;
    walk_to_play(&mut sink, local, SINK_CAPABILITIES).await;
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::M3Captured(_)
    ));
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::Negotiated { .. }
    ));
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::Play { .. }
    ));

    // act
    drop(sink);

    // assert
    assert_eq!(next_event(&mut events).await, FlowEvent::Teardown);
}

#[tokio::test]
async fn an_inbound_teardown_is_answered_before_the_socket_closes() {
    // arrange
    let (local, mut events) = source().await;
    let mut sink = Sink::connect(local).await;
    walk_to_play(&mut sink, local, SINK_CAPABILITIES).await;

    // act
    sink.send("TEARDOWN rtsp://x/wfd1.0 RTSP/1.0\r\nCSeq: 903\r\nSession: x\r\n\r\n")
        .await;
    let (_, reply) = sink.next_message().await;

    // assert
    assert!(
        reply.starts_with("RTSP/1.0 200 OK\r\nCSeq: 903\r\n"),
        "got: {reply}"
    );
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::M3Captured(_)
    ));
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::Negotiated { .. }
    ));
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::Play { .. }
    ));
    assert_eq!(next_event(&mut events).await, FlowEvent::Teardown);
}

#[tokio::test]
async fn an_lpcm_only_sink_is_cast_to_without_audio() {
    // arrange
    let (local, mut events) = source().await;
    let mut sink = Sink::connect(local).await;
    let lpcm_only = SINK_CAPABILITIES.replace("AAC 00000007 00", "LPCM 00000002 00");

    // act
    let sent = walk_to_play(&mut sink, local, &lpcm_only).await;

    // assert: the handshake completed, carrying no audio
    assert!(
        sent[3].contains("wfd_audio_codecs: none\r\n"),
        "got: {}",
        sent[3]
    );
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::M3Captured(_)
    ));
    assert_eq!(
        next_event(&mut events).await,
        FlowEvent::Negotiated {
            width: 1920,
            height: 1080,
            fps: 30,
            audio: false,
        }
    );
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::Play { .. }
    ));
}

#[tokio::test]
async fn a_sink_demanding_hdcp_fails_the_flow_and_closes() {
    // arrange
    let (local, mut events) = source().await;
    let mut sink = Sink::connect(local).await;
    let hdcp = SINK_CAPABILITIES.replace(
        "wfd_content_protection: none",
        "wfd_content_protection: HDCP2.0 port=1189",
    );

    // act: answer M1 and then M3 with the HDCP demand
    let (_, m1) = sink.next_message().await;
    sink.reply(cseq_of(&m1), "").await;
    let (_, m3) = sink.next_message().await;
    sink.reply(cseq_of(&m3), &hdcp).await;

    // assert
    assert!(matches!(
        next_event(&mut events).await,
        FlowEvent::M3Captured(_)
    ));
    let FlowEvent::Failed(reason) = next_event(&mut events).await else {
        panic!("the HDCP demand must fail the flow");
    };
    assert!(reason.contains("HDCP"), "got: {reason}");

    // the source closed its side rather than sitting on a dead handshake
    let mut buffer = [0u8; 64];
    let read = tokio::time::timeout(PATIENCE, sink.stream.read(&mut buffer))
        .await
        .expect("the source closed before the deadline")
        .expect("the socket reports the close");
    assert_eq!(read, 0, "the source should have closed the connection");
}

#[tokio::test]
async fn a_require_the_source_does_not_speak_is_refused_with_551() {
    // arrange
    let (local, _events) = source().await;
    let mut sink = Sink::connect(local).await;

    // act: answer M1, then demand a profile glint does not implement
    let (_, m1) = sink.next_message().await;
    sink.reply(cseq_of(&m1), "").await;
    sink.send("OPTIONS * RTSP/1.0\r\nCSeq: 904\r\nRequire: org.wfa.wfd9.9\r\n\r\n")
        .await;

    // assert: the 551 arrives, whether before or after M3
    let mut refusal = None;
    for _ in 0..2 {
        let (_, text) = sink.next_message().await;
        if text.starts_with("RTSP/1.0 551") {
            refusal = Some(text);
            break;
        }
    }
    let refusal = refusal.expect("the source refuses an unknown Require");
    assert_eq!(
        refusal,
        "RTSP/1.0 551 Option not supported\r\nCSeq: 904\r\n\
Unsupported: org.wfa.wfd9.9\r\n\r\n"
    );
}

#[tokio::test]
async fn a_second_sink_is_turned_away_while_one_is_being_served() {
    // A deliberate deviation from GND, which gives an extra connection its own
    // full handshake but tracks only the first, leaving it orphaned in a
    // half-alive state.
    // arrange
    let (local, _events) = source().await;
    let mut first = Sink::connect(local).await;
    let (_, m1) = first.next_message().await;
    first.reply(cseq_of(&m1), "").await;

    // act
    let mut second = TcpStream::connect(local)
        .await
        .expect("the listener still accepts");

    // assert: the second connection is closed without an RTSP word on it
    let mut buffer = [0u8; 64];
    let read = tokio::time::timeout(PATIENCE, second.read(&mut buffer))
        .await
        .expect("the extra connection was dealt with before the deadline")
        .expect("the socket reports the close");
    assert_eq!(read, 0, "an extra sink must be turned away, not served");
}

#[tokio::test]
async fn a_sink_that_speaks_no_rtsp_fails_the_flow_instead_of_being_skipped() {
    // rtsp-types is strict, and a television whose bytes it rejects is a
    // failed handshake — treating it as a skipped message would leave the
    // flow waiting forever for a message that already arrived.
    // arrange
    let (local, mut events) = source().await;
    let mut sink = Sink::connect(local).await;
    let (_, _m1) = sink.next_message().await;

    // act
    sink.send("this is not rtsp at all\r\n").await;

    // assert
    let FlowEvent::Failed(reason) = next_event(&mut events).await else {
        panic!("unparsable bytes must fail the flow");
    };
    assert!(
        reason.contains("not RTSP"),
        "the reason should name the parse failure, got: {reason}"
    );
}
