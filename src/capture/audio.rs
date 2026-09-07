//! Which sink the audio branch records.

/// `None` means follow whatever sink is default right now. That is not a
/// missing value: WirePlumber relinks a `stream.capture.sink` stream when the
/// default-sink metadata changes, so following costs no node lookup and
/// survives the user switching output mid-cast.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioSource {
    /// Declared, not consumed. The builder emits the follow-the-default
    /// clause today; pinning a sink needs a settings key that does not exist
    /// yet, and naming one without `stream.capture.sink` records the
    /// microphone instead.
    pub sink: Option<String>,
}

impl AudioSource {
    pub fn follow_default() -> Self {
        AudioSource { sink: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_the_default_sink_pins_no_sink_at_all() {
        // act
        let source = AudioSource::follow_default();
        // assert
        assert_eq!(source.sink, None);
    }
}
