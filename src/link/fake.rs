//! A scripted `P2pLink` for tests: it answers from a script and records what
//! it was asked.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{GroupId, LinkError, LinkHandle, P2pLink, Peer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkCall {
    Scan,
    Connect(Peer),
    Disconnect(LinkHandle),
    StaleGroups,
    RemoveGroup(GroupId),
}

#[derive(Debug, Default)]
pub struct FakeP2pLink {
    peers: Vec<Peer>,
    connect_results: Mutex<VecDeque<Result<LinkHandle, LinkError>>>,
    stale_groups: Mutex<Vec<GroupId>>,
    calls: Mutex<Vec<LinkCall>>,
    next_handle: Mutex<u64>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeP2pLink {
    pub fn new() -> Self {
        FakeP2pLink::default()
    }

    pub fn with_peers(mut self, peers: Vec<Peer>) -> Self {
        self.peers = peers;
        self
    }

    pub fn with_connect_results(self, results: Vec<Result<LinkHandle, LinkError>>) -> Self {
        *lock(&self.connect_results) = results.into();
        self
    }

    pub fn with_stale_groups(self, groups: Vec<GroupId>) -> Self {
        *lock(&self.stale_groups) = groups;
        self
    }

    pub fn calls(&self) -> Vec<LinkCall> {
        lock(&self.calls).clone()
    }

    fn record(&self, call: LinkCall) {
        lock(&self.calls).push(call);
    }
}

impl P2pLink for FakeP2pLink {
    async fn scan(&self) -> Result<Vec<Peer>, LinkError> {
        self.record(LinkCall::Scan);
        Ok(self.peers.clone())
    }

    /// An exhausted script means "connecting works": most tests care about
    /// what happens after a connection, not about the connection itself.
    async fn connect(&self, peer: &Peer) -> Result<LinkHandle, LinkError> {
        self.record(LinkCall::Connect(peer.clone()));
        if let Some(scripted) = lock(&self.connect_results).pop_front() {
            return scripted;
        }
        let mut next = lock(&self.next_handle);
        *next += 1;
        Ok(LinkHandle::new(*next))
    }

    async fn disconnect(&self, handle: LinkHandle) -> Result<(), LinkError> {
        self.record(LinkCall::Disconnect(handle));
        Ok(())
    }

    async fn stale_groups(&self) -> Result<Vec<GroupId>, LinkError> {
        self.record(LinkCall::StaleGroups);
        Ok(lock(&self.stale_groups).clone())
    }

    /// A removal has to stick rather than merely be recorded: the cleanup
    /// path removes every stale group and then re-reads the list.
    async fn remove_group(&self, id: GroupId) -> Result<(), LinkError> {
        self.record(LinkCall::RemoveGroup(id.clone()));
        lock(&self.stale_groups).retain(|g| g != &id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::MacAddr;

    fn peer(mac: &str, name: &str) -> Peer {
        Peer {
            mac: mac.parse().unwrap(),
            name: name.to_string(),
        }
    }

    #[tokio::test]
    async fn scan_returns_exactly_what_it_was_scripted_with() {
        // arrange
        let peers = vec![
            peer("aa:bb:cc:dd:ee:ff", "TV"),
            peer("00:11:22:33:44:55", "Beamer"),
        ];
        let link = FakeP2pLink::new().with_peers(peers.clone());
        // act
        let found = link.scan().await.unwrap();
        // assert
        assert_eq!(found, peers);
    }

    #[tokio::test]
    async fn scan_on_an_unscripted_fake_finds_nothing() {
        // arrange
        let link = FakeP2pLink::new();
        // act
        let found = link.scan().await.unwrap();
        // assert
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn connect_hands_out_a_handle_that_disconnect_accepts() {
        // The handle's value is deliberately not asserted: `LinkHandle` is
        // opaque, so what it carries is the fake's business.
        // arrange
        let link = FakeP2pLink::new();
        let target = peer("aa:bb:cc:dd:ee:ff", "TV");
        // act
        let handle = link.connect(&target).await.unwrap();
        link.disconnect(handle).await.unwrap();
        // assert
        assert_eq!(
            link.calls(),
            vec![LinkCall::Connect(target), LinkCall::Disconnect(handle)]
        );
    }

    #[tokio::test]
    async fn connect_hands_out_a_distinct_handle_each_time() {
        // arrange
        let link = FakeP2pLink::new();
        // act
        let first = link
            .connect(&peer("aa:bb:cc:dd:ee:ff", "TV"))
            .await
            .unwrap();
        let second = link
            .connect(&peer("00:11:22:33:44:55", "Beamer"))
            .await
            .unwrap();
        // assert
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn a_scripted_connect_failure_is_returned() {
        // arrange
        let mac: MacAddr = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let link =
            FakeP2pLink::new().with_connect_results(vec![Err(LinkError::PeerUnreachable(mac))]);
        // act
        let err = link
            .connect(&peer("aa:bb:cc:dd:ee:ff", "TV"))
            .await
            .unwrap_err();
        // assert
        assert_eq!(err, LinkError::PeerUnreachable(mac));
    }

    #[tokio::test]
    async fn scripted_connect_outcomes_are_consumed_in_order() {
        // arrange
        let mac: MacAddr = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let link = FakeP2pLink::new().with_connect_results(vec![
            Err(LinkError::PeerUnreachable(mac)),
            Ok(LinkHandle::new(7)),
        ]);
        let target = peer("aa:bb:cc:dd:ee:ff", "TV");
        // act
        let first = link.connect(&target).await;
        let second = link.connect(&target).await;
        // assert
        assert_eq!(first, Err(LinkError::PeerUnreachable(mac)));
        assert_eq!(second, Ok(LinkHandle::new(7)));
    }

    #[tokio::test]
    async fn stale_groups_returns_what_it_was_scripted_with() {
        // arrange
        let link =
            FakeP2pLink::new().with_stale_groups(vec![GroupId::new("g-1"), GroupId::new("g-2")]);
        // act
        let groups = link.stale_groups().await.unwrap();
        // assert
        assert_eq!(groups, vec![GroupId::new("g-1"), GroupId::new("g-2")]);
    }

    #[tokio::test]
    async fn removing_a_group_takes_it_out_of_the_next_stale_groups_answer() {
        // arrange
        let link =
            FakeP2pLink::new().with_stale_groups(vec![GroupId::new("g-1"), GroupId::new("g-2")]);
        // act
        link.remove_group(GroupId::new("g-1")).await.unwrap();
        let left = link.stale_groups().await.unwrap();
        // assert
        assert_eq!(left, vec![GroupId::new("g-2")]);
    }

    #[tokio::test]
    async fn removing_every_stale_group_empties_the_list() {
        // arrange
        let link =
            FakeP2pLink::new().with_stale_groups(vec![GroupId::new("g-1"), GroupId::new("g-2")]);
        // act
        for id in link.stale_groups().await.unwrap() {
            link.remove_group(id).await.unwrap();
        }
        // assert
        assert!(link.stale_groups().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_a_group_that_was_never_stale_is_ok() {
        // arrange
        let link = FakeP2pLink::new();
        // act
        let result = link.remove_group(GroupId::new("g-9")).await;
        // assert
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn every_call_is_recorded_in_order() {
        // arrange
        let link = FakeP2pLink::new().with_stale_groups(vec![GroupId::new("g-1")]);
        let target = peer("aa:bb:cc:dd:ee:ff", "TV");
        // act
        link.scan().await.unwrap();
        let handle = link.connect(&target).await.unwrap();
        link.remove_group(GroupId::new("g-1")).await.unwrap();
        link.disconnect(handle).await.unwrap();
        // assert
        assert_eq!(
            link.calls(),
            vec![
                LinkCall::Scan,
                LinkCall::Connect(target),
                LinkCall::RemoveGroup(GroupId::new("g-1")),
                LinkCall::Disconnect(handle),
            ]
        );
    }

    #[tokio::test]
    async fn a_fake_that_was_never_called_records_nothing() {
        // arrange
        let link = FakeP2pLink::new();
        // act
        let calls = link.calls();
        // assert
        assert!(calls.is_empty());
    }
}
