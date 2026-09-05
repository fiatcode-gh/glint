//! The xdg-desktop-portal ScreenCast implementation of `ScreenSource`.
//!
//! Nothing here is covered by an automated test: driving it means showing a
//! real picker on a real desktop, so `cargo test` must never reach it.
//! `examples/capture.rs` is where it gets exercised, by hand.

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::{PersistMode, Session};
use ashpd::enumflags2::BitFlags;

use super::{Capture, CaptureError, ScreenSource};

/// The ScreenCast interface version that first understood restore tokens.
/// Asking an older portal to persist is an error, not a no-op.
const PERSIST_SINCE_VERSION: u32 = 4;

#[derive(Default)]
pub struct PortalScreenSource {
    /// Held for the caller's lifetime, not for tidiness: closing the session
    /// invalidates the PipeWire node the pipeline is reading.
    session: Option<Session<Screencast>>,
}

impl PortalScreenSource {
    pub fn new() -> Self {
        PortalScreenSource::default()
    }
}

impl ScreenSource for PortalScreenSource {
    async fn start(&mut self, restore_token: Option<&str>) -> Result<Capture, CaptureError> {
        let proxy = Screencast::new().await?;
        // A retry needs a fresh session: the portal allows one select_sources
        // and one start per session, and refuses the second.
        let session = proxy.create_session(Default::default()).await?;

        let mut options = SelectSourcesOptions::default()
            .set_sources(BitFlags::from(SourceType::Monitor))
            .set_cursor_mode(CursorMode::Embedded)
            .set_multiple(false);
        if proxy.version() >= PERSIST_SINCE_VERSION {
            options = options
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(restore_token);
        }
        proxy.select_sources(&session, options).await?.response()?;

        // The picker appears here, not at select_sources.
        let streams = proxy
            .start(&session, None, Default::default())
            .await?
            .response()?;
        let stream = streams.streams().first().ok_or(CaptureError::NoStreams)?;

        let capture = Capture {
            node_id: stream.pipe_wire_node_id(),
            fd: proxy
                .open_pipe_wire_remote(&session, Default::default())
                .await?,
            restore_token: streams.restore_token().map(str::to_string),
        };
        self.session = Some(session);
        Ok(capture)
    }
}
