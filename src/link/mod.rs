//! The Wi-Fi Direct link a cast runs over, behind a trait so the session
//! logic is testable without a radio.

pub mod fake;

use crate::receiver::MacAddr;

/// Carries no Wi-Fi Display capability fields yet: what the platform
/// actually exposes is measured when the NetworkManager implementation
/// lands, not guessed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub mac: MacAddr,
    pub name: String,
}

/// Opaque on purpose: a caller hands it back to `disconnect` and never reads
/// into it, which leaves each implementation free to number links its own
/// way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinkHandle(u64);

impl LinkHandle {
    pub fn new(token: u64) -> Self {
        LinkHandle(token)
    }
}

/// A persistent Wi-Fi Direct group, named the way the link layer names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(id: impl Into<String>) -> Self {
        GroupId(id.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("no peer with MAC {0} answered")]
    PeerUnreachable(MacAddr),
    #[error("the link layer failed: {0}")]
    Backend(String),
}

/// Consumed through generics for the same reason as `SecretStore`: a native
/// `async fn` costs no boxing, nothing here needs `dyn P2pLink`, and the
/// allow is what lets that choice compile under `-D warnings`.
#[allow(async_fn_in_trait)]
pub trait P2pLink {
    async fn scan(&self) -> Result<Vec<Peer>, LinkError>;
    async fn connect(&self, peer: &Peer) -> Result<LinkHandle, LinkError>;
    async fn disconnect(&self, handle: LinkHandle) -> Result<(), LinkError>;
    async fn stale_groups(&self) -> Result<Vec<GroupId>, LinkError>;
    async fn remove_group(&self, id: GroupId) -> Result<(), LinkError>;
}
