//! The GStreamer pipeline description this crate builds — as a string.
//!
//! `build()` emits a `gst-launch`-style description and is pinned by snapshot
//! tests, so the shape of the pipeline is settled without constructing a
//! single GStreamer element.

pub mod build;
pub mod encoder;
pub mod runner;

use serde::{Deserialize, Serialize};

/// The H.264 encoder fallback chain (decision D8), in preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    VaH264,
    X264,
    OpenH264,
}

impl Encoder {
    /// One source for the three element names: the builder emits them and
    /// detection looks them up, so they cannot be allowed to drift apart.
    pub fn element(&self) -> &'static str {
        match self {
            Encoder::VaH264 => "vah264enc",
            Encoder::X264 => "x264enc",
            Encoder::OpenH264 => "openh264enc",
        }
    }
}

/// Where the muxed stream goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A cast. The string stops at the payloader because the destination
    /// arrives in `wfd_client_rtp_ports` over RTSP, and the RTSP layer
    /// appends the sink once it knows where to send.
    Rtp,
    /// A local recording — what the manual capture check plays in mpv.
    File(String),
}

/// Everything the pipeline string needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineSpec {
    pub encoder: Encoder,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub audio: bool,
    /// The portal stream's PipeWire node id.
    pub video_node: u32,
    /// The portal's PipeWire remote fd. Whoever owns it must outlive the
    /// pipeline: the launch string carries only the number, so closing the
    /// descriptor pulls the stream out from under a running capture.
    pub video_fd: i32,
    pub output: Output,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::build::build;
    use crate::pipeline::encoder::detect_installed;
    use gstreamer as gst;

    /// The portal heads cannot run inside a test, so they are swapped for test
    /// sources. Everything downstream of them — the caps clauses, the encoder
    /// arm, the parser, the muxer and the tail — is the builder's own text,
    /// and that is where linking succeeds or fails.
    fn without_the_portal(description: &str) -> String {
        description
            .replace(
                "pipewiresrc stream-properties=\"props,stream.capture.sink=true\" \
do-timestamp=true",
                "audiotestsrc num-buffers=1",
            )
            .replace(
                "pipewiresrc fd=40 path=42 do-timestamp=true",
                "videotestsrc num-buffers=1",
            )
    }

    fn constructible_spec(audio: bool, output: Output) -> PipelineSpec {
        PipelineSpec {
            // Whatever this host actually has. Pinning an encoder would make
            // the test fail on a machine that simply installed a different one.
            encoder: detect_installed(None).expect("no H.264 encoder on this host"),
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 8_000,
            audio,
            video_node: 42,
            video_fd: 40,
            output,
        }
    }

    /// A snapshot pins the TEXT. It cannot tell whether the text describes a
    /// graph GStreamer can build, so a caps clause the muxer refuses passes
    /// every snapshot and fails at the first real run — which is exactly how a
    /// raw-PCM audio branch once reached a user with six green snapshots
    /// behind it.
    #[test]
    fn the_built_description_is_a_graph_gstreamer_can_actually_link() {
        // arrange
        gst::init().ok();
        // act & assert
        for output in [
            Output::Rtp,
            Output::File("/tmp/glint-link-check.ts".to_string()),
        ] {
            for audio in [false, true] {
                let description =
                    without_the_portal(&build(&constructible_spec(audio, output.clone())));
                let parsed = gst::parse::launch(&description);
                assert!(
                    parsed.is_ok(),
                    "audio={audio} failed to link: {:?}\nfrom: {description}",
                    parsed.err()
                );
            }
        }
    }

    #[test]
    fn every_encoder_names_its_gstreamer_element() {
        // These three literals are the element names GStreamer knows. They feed
        // both the builder and detection, so a rename here silently changes
        // what gets detected as well as what gets emitted.
        // act & assert
        assert_eq!(Encoder::VaH264.element(), "vah264enc");
        assert_eq!(Encoder::X264.element(), "x264enc");
        assert_eq!(Encoder::OpenH264.element(), "openh264enc");
    }
}
