//! The GStreamer pipeline description this crate builds — as a string.
//!
//! There is no `gstreamer` dependency here on purpose. This unit is pure logic:
//! `build()` emits a `gst-launch`-style description and is pinned by snapshot
//! tests, so the crate compiles and tests whether or not GStreamer is installed.

use serde::{Deserialize, Serialize};

/// The H.264 encoder fallback chain (decision D8), in preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    VaH264,
    X264,
    OpenH264,
}
