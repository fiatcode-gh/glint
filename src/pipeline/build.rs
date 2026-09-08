//! Turn a `PipelineSpec` into a `gst-launch`-style description string.
//!
//! # Provenance of the encoder properties
//!
//! It is not uniform, so it is written down rather than assumed:
//!
//! - `vah264enc` and `x264enc` were MEASURED with `gst-inspect-1.0` on the
//!   development host on 2026-09-04 — see `docs/research/vaapi-encoder.md`.
//!   Both take `bitrate` in kbps.
//! - `openh264enc` is DOC-SOURCED and unverified: the plugin is not installed
//!   here. Its properties come from the official GStreamer documentation,
//!   <https://gstreamer.freedesktop.org/documentation/openh264/openh264enc.html>.
//!   Its `bitrate` is in **bits per second**, not kbps (decision D12) — hence
//!   `bitrate_scale`. Constrained Baseline has no B-frames, so it exposes no
//!   B-frame property at all.
//!
//! The snapshot tests pin the text. Only a live run can pin that the text
//! actually plays, and that is what `examples/record.rs` is for.

use crate::pipeline::{Encoder, Output, PipelineSpec};

/// Every encoder-specific name in one place, so no property spelling is
/// scattered through the builder. Where each spelling came from — and which of
/// them are measured rather than doc-sourced — is in the module header.
struct EncoderProps {
    /// The constant-bitrate switch, spelled differently by each element.
    rate_control: &'static str,
    /// Multiplier from `bitrate_kbps` to the element's own unit.
    bitrate_scale: u32,
    /// The B-frame property name, or `None` when the element has none.
    bframes: Option<&'static str>,
    /// The keyframe-interval property name.
    keyframe: &'static str,
    /// Anything else the element needs, appended verbatim.
    extra: &'static [&'static str],
}

fn props(encoder: Encoder) -> EncoderProps {
    match encoder {
        Encoder::VaH264 => EncoderProps {
            rate_control: "rate-control=cbr",
            bitrate_scale: 1,
            bframes: Some("b-frames"),
            keyframe: "key-int-max",
            extra: &[],
        },
        Encoder::X264 => EncoderProps {
            rate_control: "pass=cbr",
            bitrate_scale: 1,
            bframes: Some("bframes"),
            keyframe: "key-int-max",
            extra: &["tune=zerolatency"],
        },
        Encoder::OpenH264 => EncoderProps {
            rate_control: "rate-control=bitrate",
            bitrate_scale: 1000,
            bframes: None,
            keyframe: "gop-size",
            extra: &[],
        },
    }
}

/// Two seconds. A sink that joins late, or drops a packet, resynchronises
/// at the next keyframe, so the interval bounds how long its screen stays
/// broken.
fn keyframe_frames(fps: u32) -> u32 {
    2 * fps
}

pub fn build(spec: &PipelineSpec) -> String {
    let p = props(spec.encoder);

    let mut encoder_args = vec![
        p.rate_control.to_string(),
        format!("bitrate={}", spec.bitrate_kbps * p.bitrate_scale),
    ];
    if let Some(name) = p.bframes {
        encoder_args.push(format!("{name}=0"));
    }
    encoder_args.push(format!("{}={}", p.keyframe, keyframe_frames(spec.fps)));
    encoder_args.extend(p.extra.iter().map(|e| (*e).to_string()));

    // Why the RTP tail looks like this, at the site rather than only in the
    // tests that pin it:
    // - `alignment=7` on the muxer below: 7 transport packets are 1316 bytes,
    //   which is the payload size UDP streaming wants — GND's "force the
    //   correct alignment for UDP".
    // - no `pt=`: Wi-Fi Display requires the MP2T static assignment 33, which
    //   is already rtpmp2tpay's default, so naming it would be a second copy
    //   of the number free to drift from the first.
    // - `bind-port=16384`: what makes the M6 reply's advertised
    //   `server_port=16384-16385` honest instead of a port nothing is on.
    // - `sync=false async=false`: a live cast must not block on the receiver's
    //   clock or wait for a preroll it will never get.
    let tail = match &spec.output {
        Output::Rtp { host, port } => format!(
            "rtpmp2tpay ! udpsink host={host} port={port} bind-port=16384 \
sync=false async=false"
        ),
        Output::File(path) => format!("filesink location={path}"),
    };

    // `videorate name=rate` is not shaping anything: at matched input and
    // output rates it passes buffers straight through. It is here because its
    // `drop` property is the only dropped-frame count available — a filesink
    // pipeline posts no QoS messages for the bus to carry.
    let mut pipeline = format!(
        "pipewiresrc fd={fd} path={node} do-timestamp=true ! videoconvert ! \
videorate name=rate ! videoscale ! \
video/x-raw,width={width},height={height},framerate={fps}/1 ! \
{element} {args} ! h264parse config-interval=-1 ! \
mpegtsmux name=mux alignment=7 ! {tail}",
        fd = spec.video_fd,
        node = spec.video_node,
        width = spec.width,
        height = spec.height,
        fps = spec.fps,
        element = spec.encoder.element(),
        args = encoder_args.join(" "),
    );

    if spec.audio {
        // `stream.capture.sink=true` is what makes this branch record what is
        // playing: WirePlumber reads it and links the stream to the default
        // sink's monitor ports, relinking when the default changes. Naming a
        // sink with `target-object` INSTEAD records the default microphone —
        // measured — so that property must never appear without this one.
        //
        // AAC rather than the LPCM the Wi-Fi Display specification makes
        // mandatory, because LPCM is unreachable here (decision D15).
        // mpegtsmux accepts `audio/x-lpcm`, and no element in GStreamer
        // produces it from live audio — the only sources of that media type
        // are demuxers, for remuxing an existing transport stream. AAC-LC is
        // the specification's optional codec that every sink implements in
        // practice.
        pipeline.push_str(
            " pipewiresrc stream-properties=\"props,stream.capture.sink=true\" \
do-timestamp=true ! audioconvert ! audioresample ! \
audio/x-raw,rate=48000,channels=2 ! avenc_aac ! mux.",
        );
    }

    pipeline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Output;

    fn spec(encoder: Encoder, audio: bool) -> PipelineSpec {
        PipelineSpec {
            encoder,
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: 20_000,
            audio,
            video_node: 42,
            video_fd: 40,
            output: Output::Rtp {
                host: "192.168.1.5".to_string(),
                port: 19000,
            },
        }
    }

    fn file_spec(audio: bool) -> PipelineSpec {
        PipelineSpec {
            output: Output::File("/tmp/glint-test.ts".to_string()),
            ..spec(Encoder::VaH264, audio)
        }
    }

    const VIDEO_HEAD: &str = "pipewiresrc fd=40 path=42 do-timestamp=true ! \
videoconvert ! videorate name=rate ! videoscale ! \
video/x-raw,width=1920,height=1080,framerate=60/1 ! ";
    const VIDEO_TAIL: &str = " ! h264parse config-interval=-1 ! \
mpegtsmux name=mux alignment=7 ! rtpmp2tpay ! udpsink host=192.168.1.5 \
port=19000 bind-port=16384 sync=false async=false";
    const AUDIO_BRANCH: &str = " pipewiresrc \
stream-properties=\"props,stream.capture.sink=true\" do-timestamp=true ! \
audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! avenc_aac ! mux.";

    // ---- the six snapshots ----

    #[test]
    fn snapshot_vah264_without_audio() {
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert_eq!(
            built,
            format!(
                "{VIDEO_HEAD}vah264enc rate-control=cbr bitrate=20000 b-frames=0 \
key-int-max=120{VIDEO_TAIL}"
            )
        );
    }

    #[test]
    fn snapshot_vah264_with_audio() {
        // act
        let built = build(&spec(Encoder::VaH264, true));
        // assert
        assert_eq!(
            built,
            format!(
                "{VIDEO_HEAD}vah264enc rate-control=cbr bitrate=20000 b-frames=0 \
key-int-max=120{VIDEO_TAIL}{AUDIO_BRANCH}"
            )
        );
    }

    #[test]
    fn snapshot_x264_without_audio() {
        // act
        let built = build(&spec(Encoder::X264, false));
        // assert
        assert_eq!(
            built,
            format!(
                "{VIDEO_HEAD}x264enc pass=cbr bitrate=20000 bframes=0 key-int-max=120 \
tune=zerolatency{VIDEO_TAIL}"
            )
        );
    }

    #[test]
    fn snapshot_x264_with_audio() {
        // act
        let built = build(&spec(Encoder::X264, true));
        // assert
        assert_eq!(
            built,
            format!(
                "{VIDEO_HEAD}x264enc pass=cbr bitrate=20000 bframes=0 key-int-max=120 \
tune=zerolatency{VIDEO_TAIL}{AUDIO_BRANCH}"
            )
        );
    }

    #[test]
    fn snapshot_openh264_without_audio() {
        // act
        let built = build(&spec(Encoder::OpenH264, false));
        // assert
        assert_eq!(
            built,
            format!(
                "{VIDEO_HEAD}openh264enc rate-control=bitrate bitrate=20000000 \
gop-size=120{VIDEO_TAIL}"
            )
        );
    }

    #[test]
    fn snapshot_openh264_with_audio() {
        // act
        let built = build(&spec(Encoder::OpenH264, true));
        // assert
        assert_eq!(
            built,
            format!(
                "{VIDEO_HEAD}openh264enc rate-control=bitrate bitrate=20000000 \
gop-size=120{VIDEO_TAIL}{AUDIO_BRANCH}"
            )
        );
    }

    // ---- the properties the snapshots encode, pinned individually ----

    #[test]
    fn openh264_scales_the_bitrate_to_bits_per_second() {
        // Decision D12: openh264enc's bitrate is in bits per second, unlike the
        // kbps of vah264enc and x264enc. Without the x1000 this arm would
        // stream at a thousandth of the intended rate.
        // act
        let built = build(&spec(Encoder::OpenH264, false));
        // assert
        assert!(built.contains("bitrate=20000000"), "got: {built}");
    }

    #[test]
    fn the_two_measured_encoders_take_the_bitrate_in_kbps_unscaled() {
        // The trailing space matters: "bitrate=20000" is also a substring of
        // "bitrate=20000000", so without it a wrongly scaled measured encoder
        // would pass this test.
        // act & assert
        assert!(build(&spec(Encoder::VaH264, false)).contains("bitrate=20000 "));
        assert!(build(&spec(Encoder::X264, false)).contains("bitrate=20000 "));
    }

    #[test]
    fn the_keyframe_interval_is_two_seconds_worth_of_frames() {
        // arrange
        let mut at_30fps = spec(Encoder::VaH264, false);
        at_30fps.fps = 30;
        // act
        let built = build(&at_30fps);
        // assert
        assert!(built.contains("key-int-max=60"), "got: {built}");
        assert!(built.contains("framerate=30/1"), "got: {built}");
    }

    #[test]
    fn openh264_expresses_the_keyframe_interval_as_gop_size() {
        // arrange
        let mut at_24fps = spec(Encoder::OpenH264, false);
        at_24fps.fps = 24;
        // act
        let built = build(&at_24fps);
        // assert
        assert!(built.contains("gop-size=48"), "got: {built}");
    }

    #[test]
    fn both_measured_encoders_disable_b_frames() {
        // act & assert
        assert!(build(&spec(Encoder::VaH264, false)).contains("b-frames=0"));
        assert!(build(&spec(Encoder::X264, false)).contains("bframes=0"));
    }

    #[test]
    fn openh264_emits_no_b_frame_property() {
        // Constrained Baseline has no B-frames, so there is nothing to switch
        // off and openh264enc exposes no such property.
        // act
        let built = build(&spec(Encoder::OpenH264, false));
        // assert
        assert!(!built.contains("frames="), "got: {built}");
    }

    #[test]
    fn every_encoder_asks_for_constant_bitrate() {
        // act & assert
        assert!(build(&spec(Encoder::VaH264, false)).contains("rate-control=cbr"));
        assert!(build(&spec(Encoder::X264, false)).contains("pass=cbr"));
        assert!(build(&spec(Encoder::OpenH264, false)).contains("rate-control=bitrate"));
    }

    // ---- the two output tails ----

    #[test]
    fn the_rtp_output_sends_to_the_negotiated_host_and_port() {
        // The destination is no longer invented: the host is the accepted RTSP
        // connection's peer address and the port is M6 SETUP's client_port.
        // bind-port=16384 is what makes the M6 reply's server_port=16384-16385
        // honest rather than a number nothing is bound to.
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(
            built.ends_with(
                "rtpmp2tpay ! udpsink host=192.168.1.5 port=19000 bind-port=16384 \
sync=false async=false"
            ),
            "got: {built}"
        );
    }

    #[test]
    fn the_rtp_output_leaves_the_payload_type_at_the_mp2t_default() {
        // Wi-Fi Display requires the MP2T static assignment, 33, which is
        // already rtpmp2tpay's own default — naming it would be a second copy
        // of the number, free to drift from the default it is restating.
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(!built.contains("pt="), "got: {built}");
    }

    #[test]
    fn the_advertised_server_port_is_the_port_udpsink_actually_binds() {
        // The M6 reply advertises server_port=16384-16385 and this tail binds
        // 16384; the two live in different modules and both comments assert
        // they must agree. Nothing pinned them, so either literal could move
        // and leave glint advertising a port nothing is bound to — the same
        // defect the RTSP_PORT-versus-WFD_SOURCE_IES test exists to prevent.
        // act
        let built = build(&spec(Encoder::VaH264, false));
        let advertised = crate::wfd::flow::SERVER_PORTS
            .strip_prefix("server_port=")
            .expect("SERVER_PORTS names a server_port")
            .split('-')
            .next()
            .expect("the range names a first port");
        // assert
        assert!(
            built.contains(&format!("bind-port={advertised}")),
            "M6 advertises port {advertised}; the pipeline binds something else: {built}"
        );
    }

    #[test]
    fn the_muxer_aligns_seven_transport_packets_per_buffer() {
        // Seven 188-byte transport packets are 1316 bytes, the payload size
        // UDP streaming wants; without it the muxer emits whatever it has.
        // act & assert
        for built in [
            build(&spec(Encoder::VaH264, false)),
            build(&file_spec(false)),
        ] {
            assert!(
                built.contains("mpegtsmux name=mux alignment=7"),
                "got: {built}"
            );
        }
    }

    #[test]
    fn the_file_output_ends_at_filesink_and_never_reaches_the_payloader() {
        // act
        let built = build(&file_spec(false));
        // assert
        assert!(
            built.ends_with("filesink location=/tmp/glint-test.ts"),
            "got: {built}"
        );
        assert!(!built.contains("rtpmp2tpay"), "got: {built}");
    }

    // ---- the D10 additions ----

    #[test]
    fn the_audio_branch_carries_the_capture_sink_property() {
        // WirePlumber honours stream.capture.sink=true by linking the stream to
        // the default sink's monitor ports. Without the clause the branch
        // records the default microphone instead of what is playing.
        // act
        let built = build(&spec(Encoder::VaH264, true));
        // assert
        assert!(
            built.contains("stream-properties=\"props,stream.capture.sink=true\""),
            "got: {built}"
        );
    }

    #[test]
    fn no_output_variant_ever_emits_target_object() {
        // target-object alone links pipewiresrc to the default microphone —
        // measured. It is honoured only together with stream.capture.sink, so
        // until a pinned-sink setting exists it must never appear at all.
        // act & assert
        for built in [
            build(&spec(Encoder::VaH264, true)),
            build(&spec(Encoder::VaH264, false)),
            build(&file_spec(true)),
            build(&file_spec(false)),
        ] {
            assert!(!built.contains("target-object"), "got: {built}");
        }
    }

    #[test]
    fn the_video_chain_carries_a_named_videorate() {
        // Task 22 polls this element's `drop` property for the dropped-frame
        // count: a filesink pipeline posts no QoS messages, so the bus cannot
        // supply it. The name is how the runner finds the element.
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(built.contains("videorate name=rate"), "got: {built}");
    }

    #[test]
    fn both_sources_ask_pipewiresrc_to_timestamp_its_buffers() {
        // act
        let built = build(&spec(Encoder::VaH264, true));
        // assert
        assert_eq!(
            built.matches("do-timestamp=true").count(),
            2,
            "got: {built}"
        );
    }

    #[test]
    fn the_video_source_carries_the_portal_fd_and_node_id() {
        // The portal grants access through the fd; the node id picks the stream
        // out of that remote. Both come from one Capture value.
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(built.contains("pipewiresrc fd=40 path=42"), "got: {built}");
    }

    #[test]
    fn the_audio_branch_is_absent_when_audio_is_off() {
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(!built.contains("audioconvert"), "got: {built}");
        assert!(!built.contains("stream.capture.sink"), "got: {built}");
    }
}
