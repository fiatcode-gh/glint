//! Storage for the pairing secret of each receiver.

pub mod memory;
pub mod secret_service;

use crate::receiver::MacAddr;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the secret service request failed: {0}")]
    Service(#[from] oo7::dbus::Error),
    #[error("the stored secret for {mac} is not valid UTF-8")]
    NotUtf8 { mac: MacAddr },
}

/// Consumed through generics, never as `dyn SecretStore`: the daemon knows
/// which store it holds at compile time, so the trait can use a native
/// `async fn` and skip the boxing a dyn-compatible version would force. The
/// allow is that choice made to compile — `async_fn_in_trait` warns only
/// about the dyn-compatibility we are deliberately giving up.
#[allow(async_fn_in_trait)]
pub trait SecretStore {
    async fn get(&self, mac: MacAddr) -> Result<Option<String>, SecretError>;
    async fn set(&self, mac: MacAddr, secret: &str) -> Result<(), SecretError>;
    /// Deleting an entry that is not there is success: a caller cleaning up
    /// after a failed pairing should not have to look first.
    async fn delete(&self, mac: MacAddr) -> Result<(), SecretError>;
}
