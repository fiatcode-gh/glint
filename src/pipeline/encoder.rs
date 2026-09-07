//! Which H.264 encoder this machine can actually use.

use gstreamer as gst;
use gstreamer::ElementFactory;

use crate::pipeline::Encoder;

/// Decision D8, in preference order: hardware first, then the two software
/// encoders in descending quality.
const CHAIN: [Encoder; 3] = [Encoder::VaH264, Encoder::X264, Encoder::OpenH264];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncoderError {
    #[error(
        "no H.264 encoder is installed: install mesa-va-drivers-freeworld for \
vah264enc, gstreamer1-plugins-ugly for x264enc, or gstreamer1-plugin-openh264 \
for openh264enc"
    )]
    NoEncoder,
}

/// `available` is the seam that keeps this testable: the unit tests hand it a
/// fixed set, and the real caller hands it GStreamer's registry.
pub fn detect(
    preferred: Option<Encoder>,
    available: impl Fn(&str) -> bool,
) -> Result<Encoder, EncoderError> {
    if let Some(preferred) = preferred
        && available(preferred.element())
    {
        return Ok(preferred);
    }
    CHAIN
        .into_iter()
        .find(|candidate| available(candidate.element()))
        .ok_or(EncoderError::NoEncoder)
}

/// The registry is process-global, and `gst::init` is idempotent after the
/// first call, so every entry point that might be reached first may call it.
pub fn detect_installed(preferred: Option<Encoder>) -> Result<Encoder, EncoderError> {
    gst::init().ok();
    detect(preferred, |element| ElementFactory::find(element).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real GStreamer registry — never a portal, a screen, or an
    /// audio device, so it stays safe headless.
    #[test]
    fn the_registry_wrapper_answers_from_the_real_registry() {
        // Asserting a particular encoder would pin this test to one machine's
        // installed plugins. The machine-independent property is the one that
        // matters: whatever comes back must actually be installed, and only an
        // empty registry may produce NoEncoder.
        // arrange
        gst::init().ok();
        let installed = |encoder: &Encoder| ElementFactory::find(encoder.element()).is_some();
        let any_installed = CHAIN.iter().any(installed);
        // act
        let picked = detect_installed(None);
        // assert
        match picked {
            Ok(encoder) => assert!(installed(&encoder), "got: {encoder:?}"),
            Err(EncoderError::NoEncoder) => {
                assert!(
                    !any_installed,
                    "the registry has an encoder but none was found"
                )
            }
        }
    }

    fn only(installed: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |element| installed.contains(&element)
    }

    #[test]
    fn the_chain_prefers_the_hardware_encoder() {
        // act
        let picked = detect(None, only(&["vah264enc", "x264enc", "openh264enc"]));
        // assert
        assert_eq!(picked.unwrap(), Encoder::VaH264);
    }

    #[test]
    fn x264_is_next_when_the_hardware_encoder_is_absent() {
        // act
        let picked = detect(None, only(&["x264enc", "openh264enc"]));
        // assert
        assert_eq!(picked.unwrap(), Encoder::X264);
    }

    #[test]
    fn openh264_is_the_last_resort() {
        // act
        let picked = detect(None, only(&["openh264enc"]));
        // assert
        assert_eq!(picked.unwrap(), Encoder::OpenH264);
    }

    #[test]
    fn an_installed_preference_beats_the_chain_order() {
        // act
        let picked = detect(Some(Encoder::X264), only(&["vah264enc", "x264enc"]));
        // assert
        assert_eq!(picked.unwrap(), Encoder::X264);
    }

    #[test]
    fn a_preference_that_is_not_installed_falls_back_to_the_chain() {
        // Falling back beats failing: the setting names a wish, and a user who
        // uninstalls a plugin should still be able to cast.
        // act
        let picked = detect(Some(Encoder::OpenH264), only(&["vah264enc", "x264enc"]));
        // assert
        assert_eq!(picked.unwrap(), Encoder::VaH264);
    }

    #[test]
    fn no_encoder_at_all_names_every_package_that_would_fix_it() {
        // This message is the whole remedy a user gets. On Fedora none of the
        // three encoders ships by default, so omitting a package name leaves
        // them stuck with a failure they cannot act on.
        // act
        let message = detect(None, only(&[])).unwrap_err().to_string();
        // assert
        assert!(
            message.contains("mesa-va-drivers-freeworld"),
            "got: {message}"
        );
        assert!(
            message.contains("gstreamer1-plugins-ugly"),
            "got: {message}"
        );
        assert!(
            message.contains("gstreamer1-plugin-openh264"),
            "got: {message}"
        );
    }
}
