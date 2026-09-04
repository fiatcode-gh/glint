# R6: RTSP control channel — subclass gst-rtsp-server or hand-roll?

**VERDICT: Hand-roll the Wi-Fi Display RTSP control channel over a plain tokio TCP listener, using the `rtsp-types` crate for message parsing and serializing. The subclass route is blocked: the Rust bindings are missing two hooks that GNOME Network Displays depends on, and a third gap forces unsafe FFI for the core of the job.**

Research date: 2026-09-04. Sources read: `gstreamer-rs` main branch (GitHub mirror of gitlab.freedesktop.org/gstreamer/gstreamer-rs), GNOME Network Displays master branch (gitlab.gnome.org/GNOME/gnome-network-displays), MiracleCast master branch (github.com/albfan/miraclecast, last pushed 2026-03-10), and `rtsp-types` (github.com/sdroege/rtsp-types). The three blocking claims were verified twice: by a research agent and independently by raw fetches of the binding sources.

## Background

Wi-Fi Display (WFD, the Miracast protocol) reverses the normal RTSP roles. The sink (the TV) opens a TCP connection to the source on port 7236, and the source then sends RTSP requests to the sink over that connection: M1 OPTIONS, M3 GET_PARAMETER, M4 SET_PARAMETER, M5 trigger. After that the sink behaves like a normal RTSP client (SETUP, PLAY). GNOME Network Displays (GND) is the reference implementation of a WFD source on Linux. It makes stock gst-rtsp-server do this by subclassing it in C. The media transport (RTP/MPEG-TS over UDP out of a GStreamer pipeline) is separate from this decision either way.

## Approach A: subclass gst-rtsp-server through the Rust bindings

### What GND actually subclasses

Read from `src/wfd/` in gnome-network-displays (file line counts measured directly):

- **wfd-server.c** (248 lines) subclasses `GstRTSPServer`. Overrides the `create_client` and `client_connected` virtual functions (vfuncs — C-level overridable methods).
- **wfd-client.c** (657 lines) subclasses `GstRTSPClient`. Overrides eight vfuncs: `check_requirements`, `configure_client_media`, `handle_response`, `make_path_from_uri`, `new_session`, `params_set`, `pre_options_request`, and `send_message`. It sends the source-initiated M1/M3/M4/M5 requests by building messages with `gst_rtsp_message_init_request`, `gst_rtsp_message_add_header_by_name`, `gst_rtsp_message_set_body`, then calling `gst_rtsp_client_send_message`. A 25-second timer sends GET_PARAMETER keep-alives (M16).
- **wfd-media-factory.c** (927 lines) subclasses `GstRTSPMediaFactory`. Overrides `create_element`, `create_pipeline`, `construct`. This file is almost entirely encoding-pipeline construction, which glint needs in either approach.
- **wfd-media.c** (58 lines) subclasses `GstRTSPMedia`, overriding `setup_rtpbin`.
- **wfd-session-pool.c** (75 lines) subclasses `GstRTSPSessionPool`, overriding `create_session_id` to emit 10-character session identifiers, with a comment saying some LG TVs limit the length to 15 bytes.
- **wfd-params.c** (395 lines) is plain parameter parsing/formatting, no GObject subclassing.

Two of the client vfunc overrides matter most. `send_message` rewrites every outgoing message: it prepends `org.wfa.wfd1.0` to the `Public` header (the WFD capability marker sinks look for in the M2 OPTIONS response, which gst-rtsp-server generates internally) and strips `;timeout=30` from `Session` headers for sink compatibility. `handle_response` drives the whole M1-to-M5 state machine, because responses to source-initiated requests arrive through it.

### What the Rust bindings expose

Read from `gstreamer-rtsp-server/src/subclass/` in gstreamer-rs main:

- `RTSPServerImpl` has `create_client` and `client_connected` (subclass/rtsp_server.rs, lines 8 and 12). Covered.
- `RTSPClientImpl` (subclass/rtsp_client.rs) exposes about 31 vfuncs, including `handle_response`, `params_set`, `check_requirements`, `configure_client_media`, `make_path_from_uri`, `new_session`, and `pre_options_request`. But the `send_message` vfunc is **not** exposed: the file carries `// TODO: send_message` at lines 88 and 414. **Missing.**
- There is no session pool subclass module at all: `subclass/mod.rs` lists auth, client, media, media_factory, mount_points, server, and the ONVIF variants; no `rtsp_session_pool`. GND's `create_session_id` override has no Rust equivalent. **Missing.**
- `RTSPMediaFactoryImpl` has `create_element`, `construct`, `create_pipeline` (subclass/rtsp_media_factory.rs, lines 16-24). Covered. `RTSPMediaImpl` has `setup_rtpbin` (subclass/rtsp_media.rs, line 60). Covered.
- Sending server-initiated requests: a manual binding `RTSPClientExtManual::send_message` exists (gstreamer-rtsp-server/src/rtsp_client.rs). But the `RTSPMessage` type it takes (gstreamer-rtsp/src/rtsp_message.rs) has exactly three safe methods: `add_header`, `init_response`, `parse_auth_credentials`. There is no `init_request`, no `set_body`, no header reading. In the generated bindings, `gst_rtsp_client_send_message` is commented out with `message: /*Ignored*/`. So building M1/M3/M4/M5 requests means dropping to unsafe `gstreamer-rtsp-sys` FFI (foreign function interface) calls for the central operation of the protocol. **Gap.**

### Capability table

| Capability GND needs | In Rust bindings? | Source reference (gstreamer-rs main / GND master) |
|---|---|---|
| RTSPServer: `create_client`, `client_connected` vfuncs | Yes | gstreamer-rtsp-server/src/subclass/rtsp_server.rs lines 8, 12 |
| RTSPClient: `handle_response`, `params_set`, `check_requirements`, `new_session`, `pre_options_request`, `make_path_from_uri`, `configure_client_media` vfuncs | Yes | gstreamer-rtsp-server/src/subclass/rtsp_client.rs (RTSPClientImpl trait) |
| RTSPClient: `send_message` vfunc (rewrite outgoing headers; GND wfd-client.c uses it to advertise `org.wfa.wfd1.0`) | **No** — `// TODO: send_message`, lines 88 and 414 | gstreamer-rtsp-server/src/subclass/rtsp_client.rs |
| `gst_rtsp_client_send_message` (send M1/M3/M4/M5) | Yes, manual binding | gstreamer-rtsp-server/src/rtsp_client.rs (`RTSPClientExtManual`) |
| Build a request message: `init_request`, `set_body`, read headers | **No** — safe API has only `add_header`, `init_response`, `parse_auth_credentials`; needs unsafe FFI | gstreamer-rtsp/src/rtsp_message.rs (60 lines total) |
| RTSPSessionPool: `create_session_id` vfunc (LG TV short session identifiers) | **No** — no session pool subclass module exists | gstreamer-rtsp-server/src/subclass/mod.rs; GND src/wfd/wfd-session-pool.c |
| RTSPMediaFactory: `create_element`, `construct`, `create_pipeline` | Yes | gstreamer-rtsp-server/src/subclass/rtsp_media_factory.rs lines 16-24 |
| RTSPMedia: `setup_rtpbin` | Yes | gstreamer-rtsp-server/src/subclass/rtsp_media.rs line 60 |

Any GND-used hook missing from the bindings is a hard blocker for Approach A — and two are missing: the `send_message` vfunc and the session pool subclass. Without the `send_message` vfunc there is no exposed place to inject `org.wfa.wfd1.0` into the M2 OPTIONS response that gst-rtsp-server generates internally. Rescuing the approach means patching gstreamer-rs upstream (and waiting on a release) plus writing unsafe FFI for message construction.

## Approach B: hand-roll the fixed WFD message flow

- **Parsing and serializing is a solved problem.** `rtsp-types` 0.1.3 (crates.io, published 2024-09-06, about 407,000 downloads, MIT license) is by Sebastian Dröge, the gstreamer-rs maintainer. It implements RFC 7826 message types, a parser, a serializer, and typed headers. Its `Method` enum (src/message.rs, lines 145-165) includes `Options`, `GetParameter`, `SetParameter`, `Setup`, `Play`, `Pause`, `Teardown`, and it supports `Version::V1_0` (WFD uses RTSP 1.0). It is transport-agnostic, so it sits directly on a tokio TCP stream. Last release is 2024; that is acceptable for a frozen wire format from 1998.
- **MiracleCast shows the ceiling, not the cost.** Its `src/shared/rtsp.c` is 3,274 lines — but that is a from-scratch C RTSP parser, serializer, and transport dispatcher, exactly the part `rtsp-types` replaces. The actual WFD sink state machine is `src/ctl/ctl-sink.c` (627 lines) plus `src/ctl/wfd.c` (220 lines).
- **GND shows the source-side control logic is small.** Everything except pipeline construction — wfd-client.c (657) + wfd-params.c (395) + wfd-server.c (248) + wfd-session-pool.c (75) + wfd-media.c (58) — is about 1,430 lines of C, and a chunk of that is GObject boilerplate and fighting the framework's client-role assumptions.
- **What glint must own in approach B:** a tokio TCP listener on port 7236 with message framing over `rtsp-types` (~200 lines); the M1-M8 sequence as a linear state machine with handlers for the sink's M2 OPTIONS, M6 SETUP (parse `client_port`, start the pipeline's udpsink), M7 PLAY, M8 TEARDOWN (~350 lines); `wfd_*` parameter formatting and parsing for M3/M4 (~250 lines); M16 keep-alive timer and teardown (~100 lines). Roughly 900 lines of safe Rust, every byte of it under glint's control when a TV misbehaves.

## Complexity comparison

Approach A makes glint own: four or five GObject subclasses in Rust macro boilerplate; unsafe FFI wrappers for `gst_rtsp_message_init_request`/`set_body`/header access; an upstream gstreamer-rs patch (send_message vfunc, session pool subclassing) plus the wait for a release or a vendored fork; and all the WFD parameter logic anyway (wfd-params.c has no equivalent in any binding). In exchange it reuses gst-rtsp-server's session pool, transport negotiation, and request routing — machinery built for many clients and arbitrary mounts, where glint has exactly one sink, one fixed mount, and a pipeline that ends in udpsink regardless.

Approach B makes glint own about 900 lines of protocol logic, with parsing delegated to a maintained crate by the same GStreamer developers. It drops gst-rtsp-server entirely, which also removes the mismatch that forced GND to subclass five classes in the first place. Sink-quirk workarounds (LG session identifier length, Session header timeout suffix, `org.wfa.wfd1.0` advertising) become ordinary code instead of vfunc surgery.

## Recommendation

The decision rule for this research (set in advance) says: hand-roll if the evidence shows it is simpler overall. It does, and Approach A is additionally blocked outright by the two missing hooks. **Build the RTSP control channel by hand: tokio TCP listener on 7236, `rtsp-types` for messages, a fixed M1-M8 state machine, M16 keep-alive. Keep GStreamer for the media pipeline ending in udpsink.** Borrow GND's wfd-params.c logic and its sink-compatibility workarounds as the specification for the parameter layer. The `gstreamer-rtsp-server` crate leaves the dependency set; `rtsp-types` and tokio join it.

## What this research did NOT check

- No code was run or compiled. Everything above is from reading sources; the `RTSPClientExtManual::send_message` binding and `rtsp-types` were not exercised.
- Released gstreamer-rs crate versions were not audited separately; findings are from the main branch, which should be a superset of any release.
- GND's Wi-Fi Direct/P2P layer (nd-*.c), the mice protocol, and virtual-monitor capture were out of scope.
- `rtsp-types` header coverage for the `wfd_*` extension parameters was not verified in detail; WFD parameters travel in message bodies and generic headers, so this is low risk, but it was not proven.
- Interoperability claims (which real TVs need which quirks) are taken from GND's code comments, not from testing hardware.
- MiracleCast implements mostly the sink role in the files read (ctl-sink.c); its numbers size the protocol machinery, not a source implementation.

#miracast #research #rtsp
