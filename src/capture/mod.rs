//! Getting a screen out of the desktop portal, and naming what audio to
//! record alongside it.

pub mod audio;

use std::os::fd::OwnedFd;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("the screen capture portal failed: {0}")]
    Portal(ashpd::Error),
    #[error("the screen capture picker was cancelled")]
    Cancelled,
    #[error("the portal granted the session but returned no streams")]
    NoStreams,
}

impl From<ashpd::Error> for CaptureError {
    fn from(error: ashpd::Error) -> Self {
        match error {
            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => {
                CaptureError::Cancelled
            }
            other => CaptureError::Portal(other),
        }
    }
}

/// What one successful portal start yields.
#[derive(Debug)]
pub struct Capture {
    pub node_id: u32,
    /// The PipeWire remote the portal granted. It must outlive the pipeline:
    /// the launch string carries only the descriptor number, so dropping this
    /// closes the stream out from under a running capture.
    pub fd: OwnedFd,
    /// Fresh on every successful start, because a restore token is
    /// single-use. Overwrite the stored one with this, or the next run shows
    /// the picker again.
    pub restore_token: Option<String>,
}

/// Consumed through generics for the same reason as `SecretStore`: a native
/// `async fn` costs no boxing, nothing here needs `dyn ScreenSource`, and the
/// allow is what lets that choice compile under `-D warnings`.
#[allow(async_fn_in_trait)]
pub trait ScreenSource {
    /// A stale or absent token is not an error: the portal falls back to
    /// showing the picker, silently.
    async fn start(&mut self, restore_token: Option<&str>) -> Result<Capture, CaptureError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_picker_is_told_apart_from_a_portal_failure() {
        // The session loop will offer "try again" for one and an error for the
        // other, so collapsing both into one variant loses the distinction.
        // arrange
        let cancelled = ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled);
        // act
        let mapped = CaptureError::from(cancelled);
        // assert
        assert!(matches!(mapped, CaptureError::Cancelled), "got: {mapped:?}");
    }

    #[test]
    fn any_other_portal_failure_keeps_its_original_error() {
        // arrange
        let other = ashpd::Error::Response(ashpd::desktop::ResponseError::Other);
        // act
        let mapped = CaptureError::from(other);
        // assert
        assert!(matches!(mapped, CaptureError::Portal(_)), "got: {mapped:?}");
    }
}
