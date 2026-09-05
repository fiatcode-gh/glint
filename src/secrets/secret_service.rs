//! A `SecretStore` backed by the freedesktop Secret Service, so pairing
//! secrets live in the user's wallet instead of in glint's own config.

use oo7::dbus::{Collection, Service};

use super::{SecretError, SecretStore};
use crate::receiver::MacAddr;

pub struct SecretServiceStore {
    collection: Collection,
    /// Held for the store's whole lifetime so no operation reconnects, and
    /// declared after `collection` on purpose: fields drop in declaration
    /// order, and oo7 closes the D-Bus session only once the collection has
    /// released its own reference to it.
    _service: Service,
}

impl SecretServiceStore {
    pub async fn connect() -> Result<Self, SecretError> {
        let service = Service::new().await?;
        let collection = service.default_collection().await?;
        Ok(SecretServiceStore {
            collection,
            _service: service,
        })
    }

    /// A locked collection answers `search_items` with an empty list rather
    /// than an error, which is indistinguishable from "no such secret", so
    /// every operation unlocks first instead of trusting an empty result.
    async fn unlocked(&self) -> Result<&Collection, SecretError> {
        if self.collection.is_locked().await? {
            self.collection.unlock(None).await?;
        }
        Ok(&self.collection)
    }

    fn attributes(mac: MacAddr) -> [(&'static str, String); 2] {
        [("app", "glint".to_string()), ("mac", mac.to_string())]
    }

    fn label(mac: MacAddr) -> String {
        format!("glint pairing {mac}")
    }
}

impl SecretStore for SecretServiceStore {
    async fn get(&self, mac: MacAddr) -> Result<Option<String>, SecretError> {
        let items = self
            .unlocked()
            .await?
            .search_items(&Self::attributes(mac))
            .await?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item.secret().await?;
        String::from_utf8(secret.to_vec())
            .map(Some)
            .map_err(|_| SecretError::NotUtf8 { mac })
    }

    /// `replace: true` is the upsert: it overwrites whatever already carries
    /// these attributes instead of leaving a second entry for the same mac.
    async fn set(&self, mac: MacAddr, secret: &str) -> Result<(), SecretError> {
        self.unlocked()
            .await?
            .create_item(
                &Self::label(mac),
                &Self::attributes(mac),
                secret,
                true,
                None,
            )
            .await?;
        Ok(())
    }

    async fn delete(&self, mac: MacAddr) -> Result<(), SecretError> {
        for item in self
            .unlocked()
            .await?
            .search_items(&Self::attributes(mac))
            .await?
        {
            item.delete(None).await?;
        }
        Ok(())
    }
}
