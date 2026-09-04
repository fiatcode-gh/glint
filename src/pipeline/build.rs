//! Turn a `PipelineSpec` into a `gst-launch`-style description string.
//!
//! The string deliberately ends at `rtpmp2tpay`: the UDP destination is the
//! sink's, and it arrives in `wfd_client_rtp_ports` over RTSP in Milestone 2.
//! The RTSP layer appends the sink once it knows where to send.

use crate::pipeline::{Encoder, PipelineSpec};

/// Every encoder-specific name in one place, so no property spelling is
/// scattered through the builder.
///
/// Provenance, and it is not uniform:
///
/// - `vah264enc` and `x264enc` were MEASURED with `gst-inspect-1.0` on the
///   development host on 2026-09-04 — see `docs/research/vaapi-encoder.md`.
///   Both take `bitrate` in kbps.
/// - `openh264enc` is DOC-SOURCED and unverified: the plugin is not installed
///   here. Its properties come from the official GStreamer documentation,
///   <https://gstreamer.freedesktop.org/documentation/openh264/openh264enc.html>.
///   Its `bitrate` is in **bits per second**, not kbps (decision D12) — hence
///   `bitrate_scale`. Constrained Baseline has no B-frames, so it exposes no
///   B-frame property at all.
struct EncoderProps {
    element: &'static str,
    /// The constant-bitrate switch, spelled differently by each element.
    rate_control: &'static str,
    /// Multiplier from `bitrate_kbps` to the element's own unit.
    bitrate_scale: u32,
    /// The B-frame property name, or `None` when the element has none.
    bframes: Option<&'static str>,
    /// The keyframe-interval property name; set to two seconds of frames.
    keyframe: &'static str,
    /// Anything else the element needs, appended verbatim.
    extra: &'static [&'static str],
}

fn props(encoder: Encoder) -> EncoderProps {
    match encoder {
        Encoder::VaH264 => EncoderProps {
            element: "vah264enc",
            rate_control: "rate-control=cbr",
            bitrate_scale: 1,
            bframes: Some("b-frames"),
            keyframe: "key-int-max",
            extra: &[],
        },
        Encoder::X264 => EncoderProps {
            element: "x264enc",
            rate_control: "pass=cbr",
            bitrate_scale: 1,
            bframes: Some("bframes"),
            keyframe: "key-int-max",
            extra: &["tune=zerolatency"],
        },
        Encoder::OpenH264 => EncoderProps {
            element: "openh264enc",
            rate_control: "rate-control=bitrate",
            bitrate_scale: 1000,
            bframes: None,
            keyframe: "gop-size",
            extra: &[],
        },
    }
}

/// A two-second keyframe interval, expressed in frames.
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

    let mut pipeline = format!(
        "pipewiresrc path={node} ! videoconvert ! videoscale ! \
video/x-raw,width={width},height={height},framerate={fps}/1 ! \
{element} {args} ! h264parse config-interval=-1 ! mpegtsmux name=mux ! rtpmp2tpay",
        node = spec.pipewire_node,
        width = spec.width,
        height = spec.height,
        fps = spec.fps,
        element = p.element,
        args = encoder_args.join(" "),
    );

    if spec.audio {
        // LPCM 48 kHz 16-bit stereo is the Wi-Fi Display mandatory audio
        // codec, so every sink accepts it and the branch needs no encoder
        // element. S16BE is the byte order MPEG-TS carries LPCM in.
        pipeline.push_str(&format!(
            " pipewiresrc path={} ! audioconvert ! audioresample ! \
audio/x-raw,format=S16BE,rate=48000,channels=2 ! mux.",
            spec.audio_node
        ));
    }

    pipeline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{Encoder, PipelineSpec};

    fn spec(encoder: Encoder, audio: bool) -> PipelineSpec {
        PipelineSpec {
            encoder,
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: 20_000,
            audio,
            pipewire_node: 42,
            audio_node: 43,
        }
    }

    const VIDEO_HEAD: &str = "pipewiresrc path=42 ! videoconvert ! videoscale ! \
video/x-raw,width=1920,height=1080,framerate=60/1 ! ";
    const VIDEO_TAIL: &str =
        " ! h264parse config-interval=-1 ! mpegtsmux name=mux ! rtpmp2tpay";
    const AUDIO_BRANCH: &str = " pipewiresrc path=43 ! audioconvert ! audioresample ! \
audio/x-raw,format=S16BE,rate=48000,channels=2 ! mux.";

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
        // act & assert
        assert!(build(&spec(Encoder::VaH264, false)).contains("bitrate=20000"));
        assert!(build(&spec(Encoder::X264, false)).contains("bitrate=20000"));
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

    #[test]
    fn the_string_stops_at_the_payloader_with_no_destination() {
        // The sink's host and port arrive in wfd_client_rtp_ports over RTSP,
        // in Milestone 2. Emitting a udpsink here would mean inventing them.
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(built.ends_with("rtpmp2tpay"), "got: {built}");
        assert!(!built.contains("udpsink"), "got: {built}");
    }

    #[test]
    fn the_audio_node_is_ignored_when_audio_is_off() {
        // act
        let built = build(&spec(Encoder::VaH264, false));
        // assert
        assert!(!built.contains("path=43"), "got: {built}");
    }
}
