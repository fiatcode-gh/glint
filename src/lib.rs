//! glint — pure-logic core of the Miracast sender daemon.
//!
//! The pipeline builder emits a `gst-launch`-style description string and is
//! tested by snapshot rather than by constructing GStreamer elements, so the
//! shape of a cast is settled without a display, a network, or a portal. Only
//! `pipeline::runner` and `pipeline::encoder` reach a live GStreamer, and only
//! `capture::portal` reaches a live desktop.

pub mod capture;
pub mod config;
pub mod link;
pub mod pipeline;
pub mod receiver;
pub mod reconnect;
pub mod secrets;
pub mod session;
pub mod startup;
pub mod wfd;
