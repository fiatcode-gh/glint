//! The GStreamer pipeline description this crate builds — as a string.
//!
//! There is no `gstreamer` dependency here on purpose. This unit is pure logic:
//! `build()` emits a `gst-launch`-style description and is pinned by snapshot
//! tests, so the crate compiles and tests whether or not GStreamer is installed.

pub mod build;

use serde::{Deserialize, Serialize};

/// The H.264 encoder fallback chain (decision D8), in preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    VaH264,
    X264,
    OpenH264,
}

/// Everything the pipeline string needs. No host or port: the destination
/// comes from the sink's `wfd_client_rtp_ports` over RTSP, in Milestone 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineSpec {
    pub encoder: Encoder,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub audio: bool,
    pub pipewire_node: u32,
    /// Ignored when `audio` is false.
    pub audio_node: u32,
}
