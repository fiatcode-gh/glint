//! A `SecretStore` that keeps everything in memory, for tests and dry runs.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{SecretError, SecretStore};
use crate::receiver::MacAddr;

#[derive(Debug, Default)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<MacAddr, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        MemorySecretStore::default()
    }

    /// A poisoned lock still hands back usable data here: the map is a plain
    /// key-value store with no invariant a panicking writer could have left
    /// half-applied.
    fn entries(&self) -> MutexGuard<'_, HashMap<MacAddr, String>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SecretStore for MemorySecretStore {
    async fn get(&self, mac: MacAddr) -> Result<Option<String>, SecretError> {
        Ok(self.entries().get(&mac).cloned())
    }

    async fn set(&self, mac: MacAddr, secret: &str) -> Result<(), SecretError> {
        self.entries().insert(mac, secret.to_string());
        Ok(())
    }

    async fn delete(&self, mac: MacAddr) -> Result<(), SecretError> {
        self.entries().remove(&mac);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac() -> MacAddr {
        "aa:bb:cc:dd:ee:ff".parse().unwrap()
    }

    #[tokio::test]
    async fn a_secret_that_was_set_reads_back() {
        // arrange
        let store = MemorySecretStore::new();
        // act
        store.set(mac(), "hunter2").await.unwrap();
        let found = store.get(mac()).await.unwrap();
        // assert
        assert_eq!(found, Some("hunter2".to_string()));
    }

    #[tokio::test]
    async fn getting_a_mac_with_no_secret_is_none_rather_than_an_error() {
        // arrange
        let store = MemorySecretStore::new();
        // act
        let found = store.get(mac()).await.unwrap();
        // assert
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn setting_the_same_mac_twice_replaces_the_secret() {
        // arrange
        let store = MemorySecretStore::new();
        store.set(mac(), "first").await.unwrap();
        // act
        store.set(mac(), "second").await.unwrap();
        // assert
        assert_eq!(store.get(mac()).await.unwrap(), Some("second".to_string()));
    }

    #[tokio::test]
    async fn delete_removes_the_secret() {
        // arrange
        let store = MemorySecretStore::new();
        store.set(mac(), "hunter2").await.unwrap();
        // act
        store.delete(mac()).await.unwrap();
        // assert
        assert_eq!(store.get(mac()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn deleting_a_mac_with_no_secret_is_ok() {
        // arrange
        let store = MemorySecretStore::new();
        // act
        let result = store.delete(mac()).await;
        // assert
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn deleting_one_mac_leaves_the_others_alone() {
        // arrange
        let store = MemorySecretStore::new();
        let other: MacAddr = "00:11:22:33:44:55".parse().unwrap();
        store.set(mac(), "one").await.unwrap();
        store.set(other, "two").await.unwrap();
        // act
        store.delete(mac()).await.unwrap();
        // assert
        assert_eq!(store.get(mac()).await.unwrap(), None);
        assert_eq!(store.get(other).await.unwrap(), Some("two".to_string()));
    }
}
