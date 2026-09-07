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
use std::time::Duration;

use rtsp_types::{
    Message, Method, Request, Response, ResponseBuilder, StatusCode, Url, Version, headers,
};

use crate::wfd::modes::{Table, bit_for_mode};
use crate::wfd::negotiate::{ChosenFormat, H264Profile, negotiate};
use crate::wfd::params::{
    AudioCodec, AudioCodecs, ClientRtpPorts, ContentProtection, H264Codec, VideoFormats, WfdParam,
};

/// GND waits 500 ms after the sink connects before sending M1: some sinks race
/// their own connect and miss anything sent immediately. The exact value is
/// copied from the field, not measured from a sink.
pub const PRE_M1_DELAY: Duration = Duration::from_millis(500);

/// How long glint waits for any one awaited message before giving up.
///
/// Neither reference has such a deadline: GND hangs forever on a sink that
/// connects and goes silent, because no session exists yet so its 30 s session
/// timer cannot fire either. This is glint's addition. Ten seconds is a
/// politeness bound for a television that hesitates, not a measured property,
/// and bounding every single await is what bounds the whole handshake.
pub const REPLY_DEADLINE: Duration = Duration::from_secs(10);

/// WFD 6.5.1's session timeout minus five seconds.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(25);

/// The session dies after this long with NO inbound traffic of any kind.
/// Liveness is deliberately not tracked per message: GND records that "some
/// sinks do not reply with the correct session-id", so matching replies to the
/// requests that provoked them is a trap.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(30);

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

/// The methods glint answers, with the WFD profile token first — GND prepends
/// exactly this, and the sink reads the list to decide what it may send.
const PUBLIC_METHODS: &str =
    "org.wfa.wfd1.0, OPTIONS, GET_PARAMETER, SET_PARAMETER, SETUP, PLAY, PAUSE, TEARDOWN";

/// The RTP and RTCP ports glint's udpsink binds, quoted back in the M6 reply
/// so the advertisement matches the pipeline's own `bind-port`.
const SERVER_PORTS: &str = "server_port=16384-16385";

/// A 200 reply carrying the request's CSeq.
///
/// rtsp-types defaults 200's reason phrase to "Ok"; every reply spells it "OK"
/// because WFD sinks match the phrase as a string.
fn ok_reply(cseq: u32) -> ResponseBuilder {
    Response::builder(Version::V1_0, StatusCode::Ok)
        .reason_phrase("OK")
        .header(headers::CSEQ, cseq.to_string())
}

/// The M2 reply — what glint can be asked to do.
pub fn build_options_reply(cseq: u32) -> Vec<u8> {
    serialize(Message::from(
        ok_reply(cseq)
            .header(headers::PUBLIC, PUBLIC_METHODS)
            .build(Vec::new()),
    ))
}

/// The M6 reply: the sink's own Transport plus the ports glint binds.
///
/// No `;timeout=` on the session — GND strips it because it "seems to confuse
/// some clients", and the reference sink truncates the value at `;` anyway.
pub fn build_setup_reply(cseq: u32, session: &str, client_transport: &str) -> Vec<u8> {
    serialize(Message::from(
        ok_reply(cseq)
            .header(headers::SESSION, session)
            .header(
                headers::TRANSPORT,
                format!("{client_transport};{SERVER_PORTS}"),
            )
            .build(Vec::new()),
    ))
}

/// The M7 and M8 reply: 200 plus the session.
pub fn build_session_reply(cseq: u32, session: &str) -> Vec<u8> {
    serialize(Message::from(
        ok_reply(cseq)
            .header(headers::SESSION, session)
            .build(Vec::new()),
    ))
}

/// The M13 reply, and any other request glint acknowledges without acting.
///
/// M13 is deliberately not honoured further: the pipeline already carries a
/// two-second keyframe interval, which bounds how long a sink's picture stays
/// broken, and a real force-keyframe hook needs a Runner API that does not
/// exist yet.
pub fn build_plain_ok(cseq: u32) -> Vec<u8> {
    serialize(Message::from(ok_reply(cseq).build(Vec::new())))
}

/// The refusal for a `Require` glint does not implement.
pub fn build_unsupported_reply(cseq: u32, unsupported: &str) -> Vec<u8> {
    serialize(Message::from(
        Response::builder(Version::V1_0, StatusCode::OptionNotSupported)
            // The crate's default phrase capitalises every word; RFC 2326
            // spells it this way, and the sink compares strings.
            .reason_phrase("Option not supported")
            .header(headers::CSEQ, cseq.to_string())
            .header(headers::UNSUPPORTED, unsupported)
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

/// Where the flow is in the M1-M8 sequence. One variant per protocol position:
/// what glint is waiting for is the whole of its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowState {
    /// Connected, waiting out `PRE_M1_DELAY` before M1 goes out.
    Settling,
    AwaitingM1Reply,
    AwaitingM3Reply,
    AwaitingM4Reply,
    /// M5 sent. The sink's SETUP is the real acceptance signal for M4, because
    /// both references answer M4 with 200 before parsing a byte of it.
    AwaitingSetup,
    AwaitingPlay,
    Streaming,
    /// Torn down or failed. Terminal.
    Done,
}

/// What the driver must put on the socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    /// Already-serialized bytes, so a test asserts exactly what a sink sees.
    Bytes(Vec<u8>),
    Close,
}

/// What the flow's caller has to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEvent {
    /// The M3 reply body, verbatim. Normalising it would destroy the bytes the
    /// real-television fixture is meant to record.
    M3Captured(Vec<u8>),
    /// What the negotiation settled on, announced as soon as it is decided.
    ///
    /// `Play` deliberately carries only the destination, but the caller builds
    /// its pipeline at PLAY out of both — so the format is announced here, at
    /// the moment it is chosen, and the caller holds it until PLAY arrives.
    Negotiated {
        width: u32,
        height: u32,
        fps: u32,
        audio: bool,
    },
    /// The sink is playing: start the pipeline pointed here.
    Play {
        rtp_host: String,
        rtp_port: u16,
    },
    Teardown,
    Failed(String),
}

/// The Wi-Fi Display flow over one accepted connection.
///
/// Pure: it reads no clock and touches no socket. `on_tick` and `on_message`
/// both take the elapsed time since the connection was accepted, which is what
/// makes every deadline arithmetic a test can pin exactly.
#[derive(Debug)]
pub struct Flow {
    state: FlowState,
    /// The accepted socket's own address, for the presentation URL. The link
    /// layer exposes no IP, so the socket is the only source of truth.
    local: SocketAddr,
    /// The sink's address. Its IP is where RTP goes.
    peer: SocketAddr,
    session_seed: u64,
    cseq: u32,
    session: Option<String>,
    chosen: Option<ChosenFormat>,
    audio: Option<AudioCodecs>,
    sink_ports: Option<ClientRtpPorts>,
    rtp_port: Option<u16>,
    /// When the message glint is currently awaiting was sent.
    awaiting_since: Option<Duration>,
    /// When the last inbound message of any kind arrived. `None` until the
    /// session is armed at SETUP, which is when liveness starts mattering.
    last_inbound: Option<Duration>,
    last_keep_alive: Option<Duration>,
}

impl Flow {
    pub fn new(local: SocketAddr, peer: SocketAddr, session_seed: u64) -> Self {
        Flow {
            state: FlowState::Settling,
            local,
            peer,
            session_seed,
            cseq: 0,
            session: None,
            chosen: None,
            audio: None,
            sink_ports: None,
            rtp_port: None,
            awaiting_since: None,
            last_inbound: None,
            last_keep_alive: None,
        }
    }

    pub fn state(&self) -> FlowState {
        self.state
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// The negotiated video format, once M3 has been answered. The pipeline
    /// needs it and `FlowEvent::Play` deliberately carries only the
    /// destination, so it is read from here.
    pub fn chosen_format(&self) -> Option<ChosenFormat> {
        self.chosen
    }

    pub fn audio_selected(&self) -> bool {
        self.audio.is_some()
    }

    fn next_cseq(&mut self) -> u32 {
        self.cseq += 1;
        self.cseq
    }

    /// Send a message and start waiting for whatever answers it.
    fn send_awaiting(&mut self, bytes: Vec<u8>, next: FlowState, now: Duration) -> Vec<Outbound> {
        self.state = next;
        self.awaiting_since = Some(now);
        vec![Outbound::Bytes(bytes)]
    }

    fn fail(&mut self, reason: String) -> (Vec<Outbound>, Vec<FlowEvent>) {
        self.state = FlowState::Done;
        (vec![Outbound::Close], vec![FlowEvent::Failed(reason)])
    }

    /// Advance the timers. `now` is the time since the connection was accepted.
    pub fn on_tick(&mut self, now: Duration) -> (Vec<Outbound>, Vec<FlowEvent>) {
        if self.state == FlowState::Done {
            return (Vec::new(), Vec::new());
        }

        if self.state == FlowState::Settling {
            if now < PRE_M1_DELAY {
                return (Vec::new(), Vec::new());
            }
            let cseq = self.next_cseq();
            let out = self.send_awaiting(build_m1(cseq), FlowState::AwaitingM1Reply, now);
            return (out, Vec::new());
        }

        // Liveness first: once it has expired, a keep-alive would be shouting
        // into a session that is already gone.
        if let Some(last) = self.last_inbound
            && now.saturating_sub(last) >= SESSION_TIMEOUT
        {
            return self.fail(format!(
                "the sink sent nothing for {} seconds",
                SESSION_TIMEOUT.as_secs()
            ));
        }

        if let Some(sent) = self.awaiting_since
            && now.saturating_sub(sent) >= REPLY_DEADLINE
        {
            return self.fail(format!(
                "the sink did not answer within {} seconds",
                REPLY_DEADLINE.as_secs()
            ));
        }

        if let (Some(session), Some(last)) = (self.session.clone(), self.last_keep_alive)
            && now.saturating_sub(last) >= KEEP_ALIVE_INTERVAL
        {
            self.last_keep_alive = Some(now);
            let cseq = self.next_cseq();
            // A keep-alive is not awaited: GND treats a missing reply as
            // harmless and lets the liveness deadline be the only judge.
            return (vec![Outbound::Bytes(build_m16(cseq, &session))], Vec::new());
        }

        (Vec::new(), Vec::new())
    }

    /// Take one message from the sink.
    pub fn on_message(
        &mut self,
        message: &Message<Vec<u8>>,
        now: Duration,
    ) -> (Vec<Outbound>, Vec<FlowEvent>) {
        if self.state == FlowState::Done {
            return (Vec::new(), Vec::new());
        }

        // Any inbound message counts as liveness, once the session exists.
        if self.session.is_some() {
            self.last_inbound = Some(now);
        }

        match message {
            Message::Data(frame) => {
                // WFD never interleaves data on the control channel, so this is
                // a misbehaving sink. Logged rather than dropped silently.
                tracing::warn!(
                    channel = frame.channel_id(),
                    bytes = frame.len(),
                    "the sink interleaved a data frame on the RTSP channel"
                );
                (Vec::new(), Vec::new())
            }
            Message::Request(request) => self.on_request(request, now),
            Message::Response(response) => self.on_response(response, now),
        }
    }

    /// Give up on the flow for a reason the driver found rather than the
    /// protocol: bytes that are not RTSP at all, or a socket that failed. The
    /// state machine stays the only thing that decides the flow is over.
    pub fn abort(&mut self, reason: String) -> (Vec<Outbound>, Vec<FlowEvent>) {
        if self.state == FlowState::Done {
            return (Vec::new(), Vec::new());
        }
        self.fail(reason)
    }

    /// The socket closed. The reference sink never sends M8, so this is the
    /// normal end of a cast rather than an error.
    pub fn on_hangup(&mut self) -> Vec<FlowEvent> {
        if self.state == FlowState::Done {
            return Vec::new();
        }
        self.state = FlowState::Done;
        vec![FlowEvent::Teardown]
    }

    fn on_request(
        &mut self,
        request: &Request<Vec<u8>>,
        now: Duration,
    ) -> (Vec<Outbound>, Vec<FlowEvent>) {
        let cseq = match request
            .header(&headers::CSEQ)
            .and_then(|value| value.as_str().trim().parse::<u32>().ok())
        {
            Some(cseq) => cseq,
            None => {
                return self.fail("the sink sent a request with no usable CSeq".to_string());
            }
        };

        match request.method() {
            // M2, and any later OPTIONS. Answered from any state: the
            // reference sink sends it the moment it has replied to M1.
            Method::Options => {
                if let Some(require) = request.header(&headers::REQUIRE)
                    && !require.as_str().trim().eq_ignore_ascii_case(WFD_PROFILE)
                {
                    let unsupported = require.as_str().trim().to_string();
                    let refusal = build_unsupported_reply(cseq, &unsupported);
                    self.state = FlowState::Done;
                    return (
                        vec![Outbound::Bytes(refusal), Outbound::Close],
                        vec![FlowEvent::Failed(format!(
                            "the sink requires {unsupported}, which glint does not implement"
                        ))],
                    );
                }
                (vec![Outbound::Bytes(build_options_reply(cseq))], Vec::new())
            }
            Method::Setup => self.on_setup(request, cseq, now),
            Method::Play => self.on_play(cseq),
            Method::Teardown => {
                self.state = FlowState::Done;
                let reply = match &self.session {
                    Some(session) => build_session_reply(cseq, session),
                    None => build_plain_ok(cseq),
                };
                (
                    vec![Outbound::Bytes(reply), Outbound::Close],
                    vec![FlowEvent::Teardown],
                )
            }
            // M13 and the other sink-initiated SET_PARAMETERs, plus any
            // GET_PARAMETER the sink sends. Acknowledged and not acted on:
            // M13's recovery is already bounded by the pipeline's two-second
            // keyframe interval, and the rest carry nothing glint consumes.
            Method::SetParameter | Method::GetParameter => {
                let body = parse_body(request.body());
                if body_has(&body, "wfd_idr_request") {
                    tracing::debug!(%cseq, "the sink asked for a keyframe");
                } else if let Some((name, _)) = body.first() {
                    tracing::debug!(%cseq, %name, "the sink sent a parameter glint does not act on");
                }
                (vec![Outbound::Bytes(build_plain_ok(cseq))], Vec::new())
            }
            other => self.fail(format!("the sink sent an unexpected {other:?} request")),
        }
    }

    fn on_setup(
        &mut self,
        request: &Request<Vec<u8>>,
        cseq: u32,
        now: Duration,
    ) -> (Vec<Outbound>, Vec<FlowEvent>) {
        let Some(transport) = request
            .header(&headers::TRANSPORT)
            .map(|value| value.as_str().trim().to_string())
        else {
            return self.fail("the sink's SETUP carried no Transport header".to_string());
        };
        let Some(rtp_port) = client_port(&transport) else {
            return self.fail(format!(
                "the sink's SETUP Transport names no client port: {transport}"
            ));
        };

        // GND effectively uses the SETUP port and ignores the M3 claim, so a
        // mismatch is worth saying out loud but never worth refusing over.
        if let Some(claimed) = &self.sink_ports
            && claimed.rtp_port0 != rtp_port
        {
            tracing::warn!(
                m3_port = claimed.rtp_port0,
                setup_port = rtp_port,
                "the sink's SETUP port disagrees with its wfd_client_rtp_ports; using SETUP"
            );
        }

        let session = session_id(self.session_seed);
        let reply = build_setup_reply(cseq, &session, &transport);
        self.session = Some(session);
        self.rtp_port = Some(rtp_port);
        self.state = FlowState::AwaitingPlay;
        // The session exists from here, so both timers start now.
        self.last_inbound = Some(now);
        self.last_keep_alive = Some(now);
        self.awaiting_since = Some(now);
        (vec![Outbound::Bytes(reply)], Vec::new())
    }

    fn on_play(&mut self, cseq: u32) -> (Vec<Outbound>, Vec<FlowEvent>) {
        let (Some(session), Some(rtp_port)) = (self.session.clone(), self.rtp_port) else {
            return self.fail("the sink sent PLAY before SETUP".to_string());
        };
        self.state = FlowState::Streaming;
        self.awaiting_since = None;
        (
            vec![Outbound::Bytes(build_session_reply(cseq, &session))],
            vec![FlowEvent::Play {
                rtp_host: self.peer.ip().to_string(),
                rtp_port,
            }],
        )
    }

    fn on_response(
        &mut self,
        response: &Response<Vec<u8>>,
        now: Duration,
    ) -> (Vec<Outbound>, Vec<FlowEvent>) {
        match self.state {
            // GND never validates the M1 reply, and source-impl's strict
            // three-token check on its Public kills a spec-legal answer.
            FlowState::AwaitingM1Reply => {
                tracing::debug!(
                    status = ?response.status(),
                    public = ?response.header(&headers::PUBLIC).map(|v| v.as_str()),
                    "the sink answered M1"
                );
                let cseq = self.next_cseq();
                let out = self.send_awaiting(build_m3(cseq), FlowState::AwaitingM3Reply, now);
                (out, Vec::new())
            }
            FlowState::AwaitingM3Reply => self.on_capabilities(response, now),
            // A 200 here means nothing — both references reply before parsing
            // — so the trigger goes out and SETUP is the real signal.
            FlowState::AwaitingM4Reply => {
                tracing::debug!(status = ?response.status(), "the sink answered M4");
                let cseq = self.next_cseq();
                let out = self.send_awaiting(build_m5(cseq), FlowState::AwaitingSetup, now);
                (out, Vec::new())
            }
            // A keep-alive reply, or the sink answering M5. Its arrival is all
            // that matters: it has already reset the liveness deadline.
            _ => {
                tracing::debug!(status = ?response.status(), "the sink sent a reply glint only counts as liveness");
                (Vec::new(), Vec::new())
            }
        }
    }

    fn on_capabilities(
        &mut self,
        response: &Response<Vec<u8>>,
        now: Duration,
    ) -> (Vec<Outbound>, Vec<FlowEvent>) {
        let raw = response.body().to_vec();
        let body = parse_body(&raw);
        let captured = FlowEvent::M3Captured(raw);

        let Some(formats) = body_value(&body, "wfd_video_formats") else {
            return with(
                captured,
                self.fail("the sink's M3 reply carried no wfd_video_formats".to_string()),
            );
        };
        let sink_formats = match VideoFormats::parse(formats) {
            Ok(parsed) => parsed,
            Err(error) => {
                return with(
                    captured,
                    self.fail(format!(
                        "the sink's wfd_video_formats is unreadable: {error}"
                    )),
                );
            }
        };

        // A sink that omits content protection is not demanding it. GND never
        // even asks for the parameter.
        let protection = match body_value(&body, "wfd_content_protection") {
            Some(value) => match ContentProtection::parse(value) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return with(
                        captured,
                        self.fail(format!(
                            "the sink's wfd_content_protection is unreadable: {error}"
                        )),
                    );
                }
            },
            None => ContentProtection::None,
        };

        let chosen = match negotiate(&our_video_formats(), &sink_formats, &protection) {
            Ok(chosen) => chosen,
            Err(error) => return with(captured, self.fail(error.to_string())),
        };

        let sink_audio = match body_value(&body, "wfd_audio_codecs") {
            Some(value) => match AudioCodecs::parse(value) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return with(
                        captured,
                        self.fail(format!(
                            "the sink's wfd_audio_codecs is unreadable: {error}"
                        )),
                    );
                }
            },
            None => AudioCodecs(Vec::new()),
        };
        let audio = select_audio(&sink_audio);
        if audio.is_none() {
            tracing::warn!(
                offered = %sink_audio.format(),
                "no AAC at 48 kHz stereo on offer; casting video only"
            );
        }

        // The ports are echoed verbatim in M4 whatever they say — it is
        // protocol theatre on both sides — so an unreadable value is only
        // worth a warning, and SETUP is where the real port comes from.
        let sink_ports = body_value(&body, "wfd_client_rtp_ports")
            .and_then(|value| match ClientRtpPorts::parse(value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    tracing::warn!(%error, "the sink's wfd_client_rtp_ports is unreadable");
                    None
                }
            })
            .unwrap_or_else(|| ClientRtpPorts {
                profile: "RTP/AVP/UDP;unicast".to_string(),
                rtp_port0: 0,
                rtp_port1: 0,
                mode: "play".to_string(),
            });

        self.chosen = Some(chosen);
        self.audio = audio;
        self.sink_ports = Some(sink_ports.clone());

        let cseq = self.next_cseq();
        let m4 = build_m4(cseq, &chosen, self.audio.as_ref(), self.local, &sink_ports);
        let out = self.send_awaiting(m4, FlowState::AwaitingM4Reply, now);
        (
            out,
            vec![
                captured,
                FlowEvent::Negotiated {
                    width: chosen.width,
                    height: chosen.height,
                    fps: chosen.fps,
                    audio: self.audio.is_some(),
                },
            ],
        )
    }
}

/// Put the M3 capture in front of whatever else a step produced, so the raw
/// bytes reach the caller even when the negotiation over them fails — that
/// failure is exactly when a human most wants to see them.
fn with(
    captured: FlowEvent,
    (out, events): (Vec<Outbound>, Vec<FlowEvent>),
) -> (Vec<Outbound>, Vec<FlowEvent>) {
    let mut all = vec![captured];
    all.extend(events);
    (out, all)
}

/// The first port of a Transport header's `client_port`.
///
/// The reference sink sends a single port, but the grammar allows `n-m` and a
/// real television may send one; the first is the RTP port either way.
fn client_port(transport: &str) -> Option<u16> {
    transport
        .split(';')
        .find_map(|field| field.trim().strip_prefix("client_port="))
        .and_then(|value| value.split('-').next())
        .and_then(|digits| digits.trim().parse().ok())
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

    #[test]
    fn the_m2_reply_advertises_the_wfd_profile_first() {
        // GND prepends exactly "org.wfa.wfd1.0, " to the Public it answers
        // with, and the sink reads that list to decide what it may send.
        // act
        let built = build_options_reply(1);
        // assert
        assert_eq!(
            utf8(built),
            "RTSP/1.0 200 OK\r\n\
CSeq: 1\r\n\
Public: org.wfa.wfd1.0, OPTIONS, GET_PARAMETER, SET_PARAMETER, SETUP, PLAY, PAUSE, TEARDOWN\r\n\
\r\n"
        );
    }

    #[test]
    fn every_reply_spells_the_reason_phrase_in_capitals() {
        // rtsp-types' own default for 200 is "Ok", and WFD sinks are
        // string-matchy about it.
        for built in [
            build_options_reply(1),
            build_setup_reply(2, "abcdefghij", "RTP/AVP/UDP;unicast;client_port=19000"),
            build_session_reply(3, "abcdefghij"),
            build_plain_ok(4),
        ] {
            // assert
            let text = utf8(built);
            assert!(text.starts_with("RTSP/1.0 200 OK\r\n"), "got: {text}");
        }
    }

    #[test]
    fn the_m6_reply_echoes_the_transport_and_adds_our_server_ports() {
        // server_port has to match the udpsink bind-port the pipeline pins, or
        // the reply advertises a port nothing is bound to.
        // act
        let built = build_setup_reply(2, "abcdefghij", "RTP/AVP/UDP;unicast;client_port=19000");
        // assert
        assert_eq!(
            utf8(built),
            "RTSP/1.0 200 OK\r\n\
CSeq: 2\r\n\
Session: abcdefghij\r\n\
Transport: RTP/AVP/UDP;unicast;client_port=19000;server_port=16384-16385\r\n\
\r\n"
        );
    }

    #[test]
    fn no_reply_carries_a_session_timeout_suffix() {
        // GND suppresses it in responses as well as requests.
        for built in [
            build_setup_reply(2, "abcdefghij", "RTP/AVP/UDP;unicast;client_port=19000"),
            build_session_reply(3, "abcdefghij"),
        ] {
            // assert
            assert!(!utf8(built).contains("timeout="));
        }
    }

    #[test]
    fn the_m7_reply_carries_only_the_session() {
        // act
        let built = build_session_reply(3, "abcdefghij");
        // assert
        assert_eq!(
            utf8(built),
            "RTSP/1.0 200 OK\r\n\
CSeq: 3\r\n\
Session: abcdefghij\r\n\
\r\n"
        );
    }

    #[test]
    fn a_plain_acknowledgement_carries_no_session_at_all() {
        // M13 arrives before glint has any reason to name a session, and the
        // reference sink reads nothing but the status from it.
        // act
        let built = build_plain_ok(4);
        // assert
        assert_eq!(utf8(built), "RTSP/1.0 200 OK\r\nCSeq: 4\r\n\r\n");
    }

    #[test]
    fn a_require_glint_does_not_speak_is_answered_with_551() {
        // The crate's own default phrase here is "Option Not Supported"; RFC
        // 2326 spells it "Option not supported", and a string-matchy sink gets
        // the RFC's spelling.
        // act
        let built = build_unsupported_reply(1, "org.wfa.wfd9.9");
        // assert
        assert_eq!(
            utf8(built),
            "RTSP/1.0 551 Option not supported\r\n\
CSeq: 1\r\n\
Unsupported: org.wfa.wfd9.9\r\n\
\r\n"
        );
    }

    #[test]
    fn no_reply_ever_carries_a_content_length() {
        // Every reply glint sends has an empty body, and the reference sink's
        // own body-less replies omit the header too — so a Content-Length: 0
        // here would be glint inventing a form neither side sends.
        for built in [
            build_options_reply(1),
            build_setup_reply(2, "s", "RTP/AVP/UDP;unicast;client_port=19000"),
            build_session_reply(3, "s"),
            build_plain_ok(4),
            build_unsupported_reply(5, "x"),
        ] {
            // assert
            assert!(!utf8(built).contains("Content-Length"));
        }
    }

    /// The reference sink's default M3 reply body, from its documented values.
    const SINK_M3_REPLY: &str = "wfd_video_formats: 00 00 03 10 0001ffff 1fffffff 00001fff 00 0000 0000 00 none none\r\n\
wfd_audio_codecs: AAC 00000007 00\r\n\
wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n\
wfd_content_protection: none\r\n";

    const SINK_SETUP: &str = "SETUP rtsp://192.168.1.9:7236/wfd1.0/streamid=0 RTSP/1.0\r\n\
CSeq: 100\r\nTransport: RTP/AVP/UDP;unicast;client_port=19000\r\n\r\n";

    const SINK_PLAY: &str = "PLAY rtsp://192.168.1.9:7236/wfd1.0/streamid=0 RTSP/1.0\r\n\
CSeq: 101\r\nSession: abc\r\n\r\n";

    fn flow() -> Flow {
        Flow::new(local(), "192.168.1.20:41234".parse().unwrap(), 7)
    }

    /// The bytes a step put on the wire, concatenated.
    fn wire(out: &[Outbound]) -> String {
        out.iter()
            .filter_map(|item| match item {
                Outbound::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
                Outbound::Close => None,
            })
            .collect()
    }

    fn closed(out: &[Outbound]) -> bool {
        out.iter().any(|item| matches!(item, Outbound::Close))
    }

    fn failed(events: &[FlowEvent]) -> Option<String> {
        events.iter().find_map(|event| match event {
            FlowEvent::Failed(reason) => Some(reason.clone()),
            _ => None,
        })
    }

    /// A 200 reply from the sink, with a correct Content-Length so the whole
    /// message parses rather than leaving its body in the buffer.
    fn response(cseq: u32, body: &str) -> Message<Vec<u8>> {
        let raw = if body.is_empty() {
            format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n\r\n")
        } else {
            format!(
                "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: text/parameters\r\n\
Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        Message::parse(raw.as_bytes())
            .expect("the test fixture is a valid message")
            .0
    }

    fn request(raw: &str) -> Message<Vec<u8>> {
        Message::parse(raw.as_bytes())
            .expect("the test fixture is a valid message")
            .0
    }

    /// A flow walked to PLAY: M1 at 0.5 s, replies at 1-3 s, SETUP at 4 s and
    /// PLAY at 5 s. PLAY is inbound, so it is the liveness base.
    fn at_play() -> Flow {
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        flow.on_message(&response(3, ""), Duration::from_secs(3));
        flow.on_message(&request(SINK_SETUP), Duration::from_secs(4));
        flow.on_message(&request(SINK_PLAY), Duration::from_secs(5));
        flow
    }

    #[test]
    fn nothing_is_sent_before_the_settle_delay_elapses() {
        // GND waits 500 ms after accept because some sinks race their own
        // connect and miss anything sent immediately.
        // arrange
        let mut flow = flow();
        // act
        let (out, events) = flow.on_tick(Duration::from_millis(499));
        // assert
        assert!(out.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn m1_goes_out_once_the_settle_delay_is_up() {
        // arrange
        let mut flow = flow();
        // act
        let (out, _) = flow.on_tick(Duration::from_millis(500));
        // assert
        assert!(
            wire(&out).starts_with("OPTIONS * RTSP/1.0\r\n"),
            "{:?}",
            wire(&out)
        );
    }

    #[test]
    fn m1_is_never_sent_twice() {
        // GND sends it exactly once; a second OPTIONS would restart the
        // handshake the sink is already answering.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        // act
        let (out, _) = flow.on_tick(Duration::from_millis(600));
        // assert
        assert!(wire(&out).is_empty());
    }

    #[test]
    fn the_m1_reply_is_answered_with_m3() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        // act
        let (out, _) = flow.on_message(&response(1, ""), Duration::from_secs(1));
        // assert
        assert!(wire(&out).starts_with("GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\n"));
    }

    #[test]
    fn the_m3_reply_is_negotiated_into_m4() {
        // CEA bit 7 is the highest mode both sides claim, so 1080p30 under
        // Constrained High at the sink's own level 10.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        // act
        let (out, _) = flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        // assert
        let sent = wire(&out);
        assert!(
            sent.contains(
                "wfd_video_formats: 00 00 02 10 00000080 00000000 00000000 00 0000 0000 00 none none\r\n"
            ),
            "got: {sent}"
        );
        assert!(
            sent.contains("wfd_audio_codecs: AAC 00000001 00\r\n"),
            "got: {sent}"
        );
        assert!(
            sent.contains("wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n"),
            "got: {sent}"
        );
    }

    #[test]
    fn the_raw_m3_reply_body_is_surfaced_verbatim() {
        // The capture path for the real-TV fixture: normalising it here would
        // destroy the very bytes the fixture is meant to record.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        // act
        let (_, events) = flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        // assert
        let captured = events
            .iter()
            .find_map(|event| match event {
                FlowEvent::M3Captured(raw) => Some(raw.clone()),
                _ => None,
            })
            .expect("the M3 reply is captured");
        assert_eq!(captured, SINK_M3_REPLY.as_bytes());
    }

    #[test]
    fn the_m4_reply_is_answered_with_the_setup_trigger() {
        // A 200 to M4 proves nothing — the reference sink replies before it
        // parses — so the trigger goes out regardless and M6 is the real
        // acceptance signal.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        // act
        let (out, _) = flow.on_message(&response(3, ""), Duration::from_secs(3));
        // assert
        assert!(wire(&out).contains("wfd_trigger_method: SETUP\r\n"));
    }

    #[test]
    fn the_setup_reply_carries_a_minted_session_and_our_server_ports() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        flow.on_message(&response(3, ""), Duration::from_secs(3));
        // act
        let (out, _) = flow.on_message(&request(SINK_SETUP), Duration::from_secs(4));
        // assert
        let sent = wire(&out);
        let session = flow.session().expect("SETUP mints a session").to_string();
        assert_eq!(
            sent,
            format!(
                "RTSP/1.0 200 OK\r\nCSeq: 100\r\nSession: {session}\r\n\
Transport: RTP/AVP/UDP;unicast;client_port=19000;server_port=16384-16385\r\n\r\n"
            )
        );
    }

    #[test]
    fn play_is_answered_and_names_the_rtp_destination() {
        // The host is the accepted socket's peer, the port is M6's client_port
        // — the sink's actual listening port, not its M3 claim.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        flow.on_message(&response(3, ""), Duration::from_secs(3));
        flow.on_message(&request(SINK_SETUP), Duration::from_secs(4));
        // act
        let (out, events) = flow.on_message(&request(SINK_PLAY), Duration::from_secs(5));
        // assert
        assert!(wire(&out).starts_with("RTSP/1.0 200 OK\r\nCSeq: 101\r\n"));
        let destination = events
            .iter()
            .find_map(|event| match event {
                FlowEvent::Play { rtp_host, rtp_port } => Some((rtp_host.clone(), *rtp_port)),
                _ => None,
            })
            .expect("PLAY names the destination");
        assert_eq!(destination, ("192.168.1.20".to_string(), 19000));
    }

    #[test]
    fn the_negotiated_format_is_readable_for_the_pipeline() {
        // act
        let flow = at_play();
        // assert
        let chosen = flow.chosen_format().expect("the format is negotiated");
        assert_eq!((chosen.width, chosen.height, chosen.fps), (1920, 1080, 30));
        assert!(flow.audio_selected());
    }

    #[test]
    fn an_inbound_options_is_answered_wherever_it_arrives() {
        // The reference sink sends M2 the moment it has replied to M1, so the
        // answer cannot depend on which state the flow is in.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        // act
        let (out, _) = flow.on_message(
            &request("OPTIONS * RTSP/1.0\r\nCSeq: 50\r\nRequire: org.wfa.wfd1.0\r\n\r\n"),
            Duration::from_millis(600),
        );
        // assert
        assert!(wire(&out).contains("Public: org.wfa.wfd1.0, OPTIONS,"));
    }

    #[test]
    fn an_inbound_options_without_a_require_header_is_still_answered() {
        // arrange
        let mut flow = flow();
        // act
        let (out, events) = flow.on_message(
            &request("OPTIONS * RTSP/1.0\r\nCSeq: 50\r\n\r\n"),
            Duration::ZERO,
        );
        // assert
        assert!(wire(&out).contains("Public: org.wfa.wfd1.0, OPTIONS,"));
        assert!(failed(&events).is_none());
    }

    #[test]
    fn a_require_we_do_not_speak_is_refused_and_fails_the_flow() {
        // arrange
        let mut flow = flow();
        // act
        let (out, events) = flow.on_message(
            &request("OPTIONS * RTSP/1.0\r\nCSeq: 50\r\nRequire: org.wfa.wfd9.9\r\n\r\n"),
            Duration::ZERO,
        );
        // assert
        assert!(wire(&out).starts_with("RTSP/1.0 551 Option not supported\r\n"));
        assert!(wire(&out).contains("Unsupported: org.wfa.wfd9.9\r\n"));
        assert!(failed(&events).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn an_lpcm_only_sink_gets_a_video_only_m4() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let lpcm = SINK_M3_REPLY.replace("AAC 00000007 00", "LPCM 00000002 00");
        // act
        let (out, events) = flow.on_message(&response(2, &lpcm), Duration::from_secs(2));
        // assert
        assert!(wire(&out).contains("wfd_audio_codecs: none\r\n"));
        assert!(
            failed(&events).is_none(),
            "a video-only cast is not a failure"
        );
        assert!(!flow.audio_selected());
    }

    #[test]
    fn a_sink_demanding_hdcp_fails_the_flow_by_name() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let hdcp = SINK_M3_REPLY.replace(
            "wfd_content_protection: none",
            "wfd_content_protection: HDCP2.0 port=1189",
        );
        // act
        let (out, events) = flow.on_message(&response(2, &hdcp), Duration::from_secs(2));
        // assert
        let reason = failed(&events).expect("HDCP fails the flow");
        assert!(reason.contains("HDCP"), "got: {reason}");
        assert!(closed(&out));
    }

    #[test]
    fn a_sink_sharing_no_video_format_fails_the_flow() {
        // arrange: the sink's only mode is VESA 1920x1200p30, which glint
        // never claims — its masks are CEA-only.
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let disjoint = SINK_M3_REPLY.replace(
            "00 00 03 10 0001ffff 1fffffff 00001fff",
            "00 00 03 10 00000000 10000000 00000000",
        );
        // act
        let (out, events) = flow.on_message(&response(2, &disjoint), Duration::from_secs(2));
        // assert
        assert!(failed(&events).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn a_missing_content_protection_parameter_is_read_as_none() {
        // GND never even asks for it, so a sink that omits it must not be
        // treated as demanding HDCP.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let without = SINK_M3_REPLY.replace("wfd_content_protection: none\r\n", "");
        // act
        let (out, events) = flow.on_message(&response(2, &without), Duration::from_secs(2));
        // assert
        assert!(failed(&events).is_none());
        assert!(wire(&out).contains("wfd_video_formats: 00 00 02 10 00000080 "));
    }

    #[test]
    fn an_unparsable_m3_reply_fails_the_flow() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        // act
        let (out, events) = flow.on_message(
            &response(2, "wfd_video_formats: not hex at all\r\n"),
            Duration::from_secs(2),
        );
        // assert
        assert!(failed(&events).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn an_m3_reply_missing_the_video_formats_fails_the_flow() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        // act
        let (out, events) = flow.on_message(
            &response(2, "wfd_audio_codecs: AAC 00000007 00\r\n"),
            Duration::from_secs(2),
        );
        // assert
        assert!(failed(&events).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn vendor_parameters_in_an_m3_reply_are_ignored_not_refused() {
        // An ini file can add microsoft_* and intel_* lines to a real sink's
        // reply; a source must tolerate parameters it does not know.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let vendored = format!("microsoft_cursor: none\r\n{SINK_M3_REPLY}intel_thing: 7\r\n");
        // act
        let (out, events) = flow.on_message(&response(2, &vendored), Duration::from_secs(2));
        // assert
        assert!(failed(&events).is_none());
        assert!(wire(&out).contains("wfd_video_formats: 00 00 02 10 00000080 "));
    }

    #[test]
    fn an_inbound_teardown_is_answered_and_ends_the_flow() {
        // arrange
        let mut flow = at_play();
        // act
        let (out, events) = flow.on_message(
            &request("TEARDOWN rtsp://x/wfd1.0 RTSP/1.0\r\nCSeq: 200\r\nSession: abc\r\n\r\n"),
            Duration::from_secs(6),
        );
        // assert
        assert!(wire(&out).starts_with("RTSP/1.0 200 OK\r\nCSeq: 200\r\n"));
        assert!(closed(&out));
        assert!(events.iter().any(|e| matches!(e, FlowEvent::Teardown)));
    }

    #[test]
    fn an_idr_request_is_acknowledged_and_otherwise_ignored() {
        // The pipeline's two-second keyframe interval bounds the sink's
        // recovery; a real force-keyframe hook is future work.
        // arrange
        let mut flow = at_play();
        let body = "wfd_idr_request\r\n";
        let raw = format!(
            "SET_PARAMETER rtsp://x/wfd1.0 RTSP/1.0\r\nCSeq: 201\r\n\
Content-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        // act
        let (out, events) = flow.on_message(&request(&raw), Duration::from_secs(6));
        // assert
        assert_eq!(wire(&out), "RTSP/1.0 200 OK\r\nCSeq: 201\r\n\r\n");
        assert!(failed(&events).is_none());
        assert!(!closed(&out));
    }

    #[test]
    fn a_hangup_ends_the_flow_the_way_a_teardown_does() {
        // The reference sink NEVER sends M8; a TCP hangup is the field norm.
        // arrange
        let mut flow = at_play();
        // act
        let events = flow.on_hangup();
        // assert
        assert!(events.iter().any(|e| matches!(e, FlowEvent::Teardown)));
    }

    #[test]
    fn a_hangup_after_teardown_says_nothing_twice() {
        // arrange
        let mut flow = at_play();
        flow.on_message(
            &request("TEARDOWN rtsp://x/wfd1.0 RTSP/1.0\r\nCSeq: 200\r\n\r\n"),
            Duration::from_secs(6),
        );
        // act
        let events = flow.on_hangup();
        // assert
        assert!(events.is_empty());
    }

    #[test]
    fn a_data_frame_is_surfaced_and_changes_nothing() {
        // WFD never interleaves data on the control channel, so a frame here
        // means a misbehaving sink — dropping it silently would hide that.
        // arrange
        let mut flow = at_play();
        let raw = [b'$', 0x00, 0x00, 0x02, 0xaa, 0xbb];
        let frame = Message::parse(&raw).expect("a data frame parses").0;
        // act
        let (out, events) = flow.on_message(&frame, Duration::from_secs(6));
        // assert
        assert!(out.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn a_silent_sink_fails_the_flow_at_the_reply_deadline() {
        // GND has no per-request timeout and hangs forever on a sink that
        // connects and then says nothing.
        // arrange: M1 is out at 500 ms and awaiting its reply
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        let deadline = Duration::from_millis(500) + REPLY_DEADLINE;
        // act
        let (_, before) = flow.on_tick(deadline - Duration::from_millis(1));
        let (out, after) = flow.on_tick(deadline);
        // assert
        assert!(failed(&before).is_none());
        assert!(failed(&after).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn the_keep_alive_goes_out_every_twenty_five_seconds_after_setup() {
        // WFD 6.5.1's session timeout minus five. The CSeq is 5 because M1,
        // M3, M4 and M5 took 1 through 4.
        // arrange: SETUP armed the timers at 4 s
        let mut flow = at_play();
        let due = Duration::from_secs(4) + KEEP_ALIVE_INTERVAL;
        // act
        let (early, _) = flow.on_tick(due - Duration::from_millis(1));
        let (m16, _) = flow.on_tick(due);
        // assert
        assert!(wire(&early).is_empty());
        assert_eq!(
            wire(&m16),
            format!(
                "GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 5\r\nSession: {}\r\n\r\n",
                flow.session().expect("the session is armed")
            )
        );
    }

    #[test]
    fn a_session_with_no_inbound_traffic_dies_at_the_session_timeout() {
        // arrange: PLAY at 5 s is the last inbound message, so it is the base
        let mut flow = at_play();
        let deadline = Duration::from_secs(5) + SESSION_TIMEOUT;
        // act
        let (_, alive) = flow.on_tick(deadline - Duration::from_millis(1));
        let (out, dead) = flow.on_tick(deadline);
        // assert
        assert!(failed(&alive).is_none());
        assert!(failed(&dead).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn any_inbound_traffic_postpones_the_liveness_deadline() {
        // GND tracks liveness per SESSION, not per message, because "some
        // sinks do not reply with the correct session-id" — so ANY inbound
        // message resets it. Without the reset, 35 s would already be dead.
        // arrange: a keep-alive reply lands at 20 s
        let mut flow = at_play();
        flow.on_message(&response(5, ""), Duration::from_secs(20));
        // act
        let (_, at_35) = flow.on_tick(Duration::from_secs(35));
        let (_, at_51) = flow.on_tick(Duration::from_secs(51));
        // assert: 15 s after the traffic it is alive, 31 s after it is dead
        assert!(failed(&at_35).is_none(), "the deadline was not reset");
        assert!(failed(&at_51).is_some());
    }

    #[test]
    fn before_setup_it_is_the_reply_deadline_that_bounds_the_handshake() {
        // No session exists until SETUP, so liveness is not armed yet and the
        // reply deadline is the only bound. Arming liveness earlier would
        // double-bound the handshake and report the wrong reason for a stall
        // — GND's own gap is that neither bound exists there at all.
        // arrange: M1 out at 500 ms, then long past BOTH deadlines
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        // act
        let (out, events) = flow.on_tick(SESSION_TIMEOUT + Duration::from_secs(1));
        // assert
        let reason = failed(&events).expect("the handshake is bounded before SETUP");
        assert!(
            reason.contains("did not answer"),
            "the reply deadline should be the reason, got: {reason}"
        );
        assert!(closed(&out));
        assert_eq!(flow.session(), None);
    }

    #[test]
    fn a_finished_flow_stays_quiet_forever() {
        // arrange
        let mut flow = at_play();
        flow.on_message(
            &request("TEARDOWN rtsp://x/wfd1.0 RTSP/1.0\r\nCSeq: 200\r\n\r\n"),
            Duration::from_secs(6),
        );
        // act
        let (out, events) = flow.on_tick(Duration::from_secs(600));
        // assert
        assert!(out.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn a_transport_naming_a_port_range_takes_the_first_port() {
        // The reference sink sends a single port, but the header's grammar
        // allows a range and a real television may well send one.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        flow.on_message(&response(3, ""), Duration::from_secs(3));
        let setup = SINK_SETUP.replace("client_port=19000", "client_port=19000-19001");
        flow.on_message(&request(&setup), Duration::from_secs(4));
        // act
        let (_, events) = flow.on_message(&request(SINK_PLAY), Duration::from_secs(5));
        // assert
        let port = events.iter().find_map(|event| match event {
            FlowEvent::Play { rtp_port, .. } => Some(*rtp_port),
            _ => None,
        });
        assert_eq!(port, Some(19000));
    }

    #[test]
    fn a_setup_without_a_usable_transport_fails_the_flow() {
        // Without a client port there is nowhere to send RTP, and inventing
        // one would stream into the void.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        flow.on_message(&response(3, ""), Duration::from_secs(3));
        let setup = SINK_SETUP.replace(
            "Transport: RTP/AVP/UDP;unicast;client_port=19000",
            "Transport: RTP/AVP/UDP;unicast",
        );
        // act
        let (out, events) = flow.on_message(&request(&setup), Duration::from_secs(4));
        // assert
        assert!(failed(&events).is_some());
        assert!(closed(&out));
    }

    #[test]
    fn an_abort_closes_the_flow_and_names_the_reason() {
        // The driver calls this when the bytes are not RTSP at all, which the
        // protocol has no message for.
        // arrange
        let mut flow = at_play();
        // act
        let (out, events) = flow.abort("the peer sent bytes that are not RTSP".to_string());
        // assert
        assert!(closed(&out));
        assert_eq!(
            failed(&events).as_deref(),
            Some("the peer sent bytes that are not RTSP")
        );
    }

    #[test]
    fn an_abort_after_the_flow_is_over_says_nothing() {
        // arrange
        let mut flow = at_play();
        flow.on_hangup();
        // act
        let (out, events) = flow.abort("too late".to_string());
        // assert
        assert!(out.is_empty());
        assert!(events.is_empty());
    }

    #[test]
    fn the_negotiated_format_is_announced_when_it_is_decided() {
        // The caller builds its pipeline at PLAY, but PLAY carries only the
        // destination — so the format has to reach it when it is settled.
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        // act
        let (_, events) = flow.on_message(&response(2, SINK_M3_REPLY), Duration::from_secs(2));
        // assert
        assert!(events.contains(&FlowEvent::Negotiated {
            width: 1920,
            height: 1080,
            fps: 30,
            audio: true,
        }));
    }

    #[test]
    fn the_announcement_says_so_when_there_is_no_audio() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let lpcm = SINK_M3_REPLY.replace("AAC 00000007 00", "LPCM 00000002 00");
        // act
        let (_, events) = flow.on_message(&response(2, &lpcm), Duration::from_secs(2));
        // assert
        assert!(events.contains(&FlowEvent::Negotiated {
            width: 1920,
            height: 1080,
            fps: 30,
            audio: false,
        }));
    }

    #[test]
    fn a_failed_negotiation_announces_no_format() {
        // arrange
        let mut flow = flow();
        flow.on_tick(Duration::from_millis(500));
        flow.on_message(&response(1, ""), Duration::from_secs(1));
        let hdcp = SINK_M3_REPLY.replace(
            "wfd_content_protection: none",
            "wfd_content_protection: HDCP2.0 port=1189",
        );
        // act
        let (_, events) = flow.on_message(&response(2, &hdcp), Duration::from_secs(2));
        // assert
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, FlowEvent::Negotiated { .. }))
        );
    }
}
