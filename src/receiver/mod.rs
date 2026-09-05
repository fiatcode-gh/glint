//! The receiver (sink) a cast targets, and its persisted form.

pub mod quirks;
pub mod registry;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 48-bit MAC address.
///
/// A local newtype rather than a dependency: the crate needs exactly one
/// canonical text form (lowercase, colon-separated) and nothing else that a
/// MAC-address crate would bring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MacAddr([u8; 6]);

impl From<[u8; 6]> for MacAddr {
    fn from(octets: [u8; 6]) -> Self {
        MacAddr(octets)
    }
}

impl MacAddr {
    pub fn octets(&self) -> [u8; 6] {
        self.0
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let o = &self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            o[0], o[1], o[2], o[3], o[4], o[5]
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MacAddrParseError {
    #[error("a MAC address needs 6 colon-separated octets, found {0}")]
    WrongOctetCount(usize),
    #[error("MAC address octet {0:?} is not two hexadecimal digits")]
    NotHex(String),
}

impl FromStr for MacAddr {
    type Err = MacAddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let fields: Vec<&str> = s.split(':').collect();
        if fields.len() != 6 {
            return Err(MacAddrParseError::WrongOctetCount(fields.len()));
        }
        let mut octets = [0u8; 6];
        for (slot, field) in octets.iter_mut().zip(fields) {
            // `u8::from_str_radix` accepts a leading sign, so check the shape
            // first rather than trusting the parse to reject "+1".
            if field.len() != 2 || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(MacAddrParseError::NotHex(field.to_string()));
            }
            *slot = u8::from_str_radix(field, 16)
                .map_err(|_| MacAddrParseError::NotHex(field.to_string()))?;
        }
        Ok(MacAddr(octets))
    }
}

impl Serialize for MacAddr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for MacAddr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// How the receiver is used: a copy of an existing output, or a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Mirror,
    Extend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

/// A known receiver, as persisted in the receivers TOML file.
///
/// Field order here is presentational only — TOML is name-keyed, so reordering
/// these fields cannot change what round-trips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receiver {
    pub mac: MacAddr,
    pub name: String,
    pub last_mode: Option<Mode>,
    pub last_resolution: Option<Resolution>,
    pub restore_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Receiver {
        // arrange
        Receiver {
            mac: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
            name: "Living Room TV".to_string(),
            last_mode: Some(Mode::Mirror),
            last_resolution: Some(Resolution {
                width: 1920,
                height: 1080,
            }),
            restore_token: Some("tok-123".to_string()),
        }
    }

    #[test]
    fn mac_addr_displays_in_canonical_lowercase_colon_form() {
        // arrange
        let mac = MacAddr::from([0xAA, 0x0B, 0xCC, 0xDD, 0xEE, 0xFF]);
        // act
        let rendered = mac.to_string();
        // assert
        assert_eq!(rendered, "aa:0b:cc:dd:ee:ff");
    }

    #[test]
    fn mac_addr_round_trips_through_its_string_form() {
        // arrange
        let text = "01:23:45:67:89:ab";
        // act
        let parsed: MacAddr = text.parse().unwrap();
        // assert
        assert_eq!(parsed.to_string(), text);
    }

    #[test]
    fn mac_addr_parsing_is_case_insensitive_but_formats_lowercase() {
        // act
        let parsed: MacAddr = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        // assert
        assert_eq!(parsed.to_string(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn mac_addr_rejects_wrong_octet_count() {
        // act
        let err = "aa:bb:cc:dd:ee".parse::<MacAddr>().unwrap_err();
        // assert
        assert_eq!(err, MacAddrParseError::WrongOctetCount(5));
    }

    #[test]
    fn mac_addr_rejects_non_hex_octets() {
        // act
        let err = "aa:bb:cc:dd:ee:zz".parse::<MacAddr>().unwrap_err();
        // assert
        assert_eq!(err, MacAddrParseError::NotHex("zz".to_string()));
    }

    #[test]
    fn mac_addr_rejects_a_signed_octet() {
        // `u8::from_str_radix` accepts a leading '+', so this case needs an
        // explicit hex-digit check rather than relying on the parse error.
        // act
        let err = "aa:bb:cc:dd:ee:+1".parse::<MacAddr>().unwrap_err();
        // assert
        assert_eq!(err, MacAddrParseError::NotHex("+1".to_string()));
    }

    #[test]
    fn receiver_round_trips_through_toml() {
        // arrange
        let original = sample();
        // act
        let text = toml::to_string(&original).unwrap();
        let decoded: Receiver = toml::from_str(&text).unwrap();
        // assert
        assert_eq!(decoded, original);
    }

    #[test]
    fn receiver_serialises_its_mac_as_the_canonical_string() {
        // act
        let text = toml::to_string(&sample()).unwrap();
        // assert
        assert!(text.contains(r#"mac = "aa:bb:cc:dd:ee:ff""#), "got: {text}");
    }

    #[test]
    fn the_mode_wire_spellings_are_the_receivers_file_format() {
        // These two literals ARE the on-disk format. A round-trip test cannot
        // catch a rename, because the serialiser and the deserialiser move
        // together — only the spelled-out strings pin them.
        // arrange
        let cases = [("mirror", Mode::Mirror), ("extend", Mode::Extend)];
        for (spelling, mode) in cases {
            let line = format!("last_mode = \"{spelling}\"");
            let text = format!("mac = \"aa:bb:cc:dd:ee:ff\"\nname = \"Living Room TV\"\n{line}\n");
            // act
            let read: Receiver = toml::from_str(&text).unwrap();
            let written = toml::to_string(&Receiver {
                last_mode: Some(mode),
                ..sample()
            })
            .unwrap();
            // assert
            assert_eq!(read.last_mode, Some(mode), "reading {spelling}");
            assert!(
                written.contains(&line),
                "writing {spelling}, got: {written}"
            );
        }
    }

    #[test]
    fn receiver_round_trips_with_every_optional_field_absent() {
        // arrange
        let original = Receiver {
            mac: "00:00:00:00:00:00".parse().unwrap(),
            name: "Bare".to_string(),
            last_mode: None,
            last_resolution: None,
            restore_token: None,
        };
        // act
        let decoded: Receiver = toml::from_str(&toml::to_string(&original).unwrap()).unwrap();
        // assert
        assert_eq!(decoded, original);
    }
}
