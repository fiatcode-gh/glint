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
