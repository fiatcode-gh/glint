//! The set of known receivers, as persisted in the receivers TOML file.

use serde::{Deserialize, Serialize};

use super::{MacAddr, Receiver};

/// The field name is the wire format: it is what makes the file an array of
/// `[[receivers]]` tables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    receivers: Vec<Receiver>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    pub fn get(&self, mac: MacAddr) -> Option<&Receiver> {
        self.receivers.iter().find(|r| r.mac == mac)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Receiver> {
        self.receivers.iter()
    }

    pub fn len(&self) -> usize {
        self.receivers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.receivers.is_empty()
    }

    /// Replacing in place rather than remove-then-push keeps the file order
    /// stable when a known receiver is paired again.
    pub fn upsert(&mut self, receiver: Receiver) {
        match self.receivers.iter_mut().find(|r| r.mac == receiver.mac) {
            Some(existing) => *existing = receiver,
            None => self.receivers.push(receiver),
        }
    }

    pub fn remove(&mut self, mac: MacAddr) -> bool {
        let before = self.receivers.len();
        self.receivers.retain(|r| r.mac != mac);
        self.receivers.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::{Mode, Resolution};

    fn sample(mac: &str, name: &str) -> Receiver {
        Receiver {
            mac: mac.parse().unwrap(),
            name: name.to_string(),
            last_mode: Some(Mode::Mirror),
            last_resolution: Some(Resolution {
                width: 1920,
                height: 1080,
            }),
            restore_token: None,
        }
    }

    #[test]
    fn a_new_registry_is_empty() {
        // act
        let registry = Registry::new();
        // assert
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn upsert_inserts_a_receiver_that_is_not_known_yet() {
        // arrange
        let mut registry = Registry::new();
        // act
        registry.upsert(sample("aa:bb:cc:dd:ee:ff", "Living Room TV"));
        // assert
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry
                .get("aa:bb:cc:dd:ee:ff".parse().unwrap())
                .unwrap()
                .name,
            "Living Room TV"
        );
    }

    #[test]
    fn upsert_on_a_known_mac_replaces_the_entry_without_duplicating_it() {
        // arrange
        let mut registry = Registry::new();
        registry.upsert(sample("aa:bb:cc:dd:ee:ff", "Old Name"));
        // act
        registry.upsert(sample("aa:bb:cc:dd:ee:ff", "New Name"));
        // assert
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry
                .get("aa:bb:cc:dd:ee:ff".parse().unwrap())
                .unwrap()
                .name,
            "New Name"
        );
    }

    #[test]
    fn upsert_replaces_in_place_and_keeps_the_surrounding_order() {
        // arrange
        let mut registry = Registry::new();
        registry.upsert(sample("00:00:00:00:00:01", "First"));
        registry.upsert(sample("00:00:00:00:00:02", "Second"));
        registry.upsert(sample("00:00:00:00:00:03", "Third"));
        // act
        registry.upsert(sample("00:00:00:00:00:02", "Second Renamed"));
        // assert
        let names: Vec<&str> = registry.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["First", "Second Renamed", "Third"]);
    }

    #[test]
    fn remove_reports_that_it_removed_a_known_receiver() {
        // arrange
        let mut registry = Registry::new();
        registry.upsert(sample("aa:bb:cc:dd:ee:ff", "Living Room TV"));
        // act
        let removed = registry.remove("aa:bb:cc:dd:ee:ff".parse().unwrap());
        // assert
        assert!(removed);
        assert!(registry.is_empty());
    }

    #[test]
    fn remove_reports_that_an_unknown_receiver_was_not_there() {
        // arrange
        let mut registry = Registry::new();
        // act
        let removed = registry.remove("aa:bb:cc:dd:ee:ff".parse().unwrap());
        // assert
        assert!(!removed);
    }

    #[test]
    fn remove_leaves_every_other_receiver_in_place() {
        // arrange
        let mut registry = Registry::new();
        registry.upsert(sample("00:00:00:00:00:01", "First"));
        registry.upsert(sample("00:00:00:00:00:02", "Second"));
        registry.upsert(sample("00:00:00:00:00:03", "Third"));
        // act
        registry.remove("00:00:00:00:00:02".parse().unwrap());
        // assert
        let names: Vec<&str> = registry.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["First", "Third"]);
    }

    #[test]
    fn get_on_an_unknown_mac_is_none() {
        // arrange
        let registry = Registry::new();
        // act
        let found = registry.get("aa:bb:cc:dd:ee:ff".parse().unwrap());
        // assert
        assert!(found.is_none());
    }

    #[test]
    fn the_receivers_file_is_an_array_of_receivers_tables() {
        // The literal header IS the on-disk format; a round-trip cannot catch
        // a rename because the serialiser and the deserialiser move together.
        // arrange
        let mut registry = Registry::new();
        registry.upsert(sample("aa:bb:cc:dd:ee:ff", "Living Room TV"));
        // act
        let text = toml::to_string(&registry).unwrap();
        // assert
        assert!(text.contains("[[receivers]]"), "got: {text}");
    }
}
