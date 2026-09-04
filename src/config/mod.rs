//! User settings, as persisted in the settings TOML file.

use serde::{Deserialize, Serialize};

use crate::pipeline::Encoder;

const DEFAULT_RETRY_TIMEOUT_SECS: u32 = 30;

fn default_audio_follows_screen() -> bool {
    true
}

fn default_retry_timeout_secs() -> u32 {
    DEFAULT_RETRY_TIMEOUT_SECS
}

/// Every field carries its own `serde` default, deliberately, rather than the
/// container-level `#[serde(default)]`. A settings file that sets one key must
/// leave every other key at its default, and per-field defaults are what make
/// that true field by field instead of all-or-nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub preferred_encoder: Option<Encoder>,
    #[serde(default)]
    pub bitrate_cap_kbps: Option<u32>,
    #[serde(default = "default_audio_follows_screen")]
    pub audio_follows_screen: bool,
    #[serde(default = "default_retry_timeout_secs")]
    pub retry_timeout_secs: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            preferred_encoder: None,
            bitrate_cap_kbps: None,
            audio_follows_screen: default_audio_follows_screen(),
            retry_timeout_secs: default_retry_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Encoder;

    #[test]
    fn defaults_match_the_contract() {
        // act
        let settings = Settings::default();
        // assert
        assert_eq!(settings.preferred_encoder, None);
        assert_eq!(settings.bitrate_cap_kbps, None);
        assert!(settings.audio_follows_screen);
        assert_eq!(settings.retry_timeout_secs, 30);
    }

    #[test]
    fn an_empty_toml_file_yields_the_defaults() {
        // act
        let settings: Settings = toml::from_str("").unwrap();
        // assert
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn setting_only_the_retry_timeout_leaves_every_other_field_at_default() {
        // act
        let settings: Settings = toml::from_str("retry_timeout_secs = 90").unwrap();
        // assert
        assert_eq!(settings.retry_timeout_secs, 90);
        assert!(settings.audio_follows_screen);
        assert_eq!(settings.preferred_encoder, None);
        assert_eq!(settings.bitrate_cap_kbps, None);
    }

    #[test]
    fn setting_only_audio_follows_screen_leaves_every_other_field_at_default() {
        // act
        let settings: Settings = toml::from_str("audio_follows_screen = false").unwrap();
        // assert
        assert!(!settings.audio_follows_screen);
        assert_eq!(settings.retry_timeout_secs, 30);
        assert_eq!(settings.preferred_encoder, None);
        assert_eq!(settings.bitrate_cap_kbps, None);
    }

    #[test]
    fn setting_only_the_preferred_encoder_leaves_every_other_field_at_default() {
        // act
        let settings: Settings = toml::from_str(r#"preferred_encoder = "x264""#).unwrap();
        // assert
        assert_eq!(settings.preferred_encoder, Some(Encoder::X264));
        assert_eq!(settings.retry_timeout_secs, 30);
        assert!(settings.audio_follows_screen);
        assert_eq!(settings.bitrate_cap_kbps, None);
    }

    #[test]
    fn setting_only_the_bitrate_cap_leaves_every_other_field_at_default() {
        // act
        let settings: Settings = toml::from_str("bitrate_cap_kbps = 12000").unwrap();
        // assert
        assert_eq!(settings.bitrate_cap_kbps, Some(12000));
        assert_eq!(settings.retry_timeout_secs, 30);
        assert!(settings.audio_follows_screen);
        assert_eq!(settings.preferred_encoder, None);
    }

    #[test]
    fn settings_round_trip_through_toml() {
        // arrange
        let original = Settings {
            preferred_encoder: Some(Encoder::VaH264),
            bitrate_cap_kbps: Some(8000),
            audio_follows_screen: false,
            retry_timeout_secs: 45,
        };
        // act
        let decoded: Settings = toml::from_str(&toml::to_string(&original).unwrap()).unwrap();
        // assert
        assert_eq!(decoded, original);
    }
}
