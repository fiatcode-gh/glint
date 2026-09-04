//! glint — pure-logic core of the Miracast sender daemon.
//!
//! This crate deliberately has **no GStreamer dependency**. The pipeline
//! builder emits a `gst-launch`-style description string and is tested by
//! snapshot, not by constructing GStreamer elements, so the whole crate builds
//! and tests on a machine with no GStreamer, no display and no network.

pub mod config;
pub mod pipeline;
pub mod receiver;
pub mod reconnect;
pub mod session;
pub mod wfd;

#[cfg(test)]
mod skeleton_tests {
    #[test]
    fn crate_builds_and_tests_run() {
        assert_eq!(env!("CARGO_PKG_NAME"), "glint");
    }
}
