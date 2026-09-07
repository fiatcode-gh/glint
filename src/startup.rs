//! What glint does before it casts anything: clear away the Wi-Fi Direct
//! groups a previous run left behind.

use crate::link::{LinkError, P2pLink};

/// Removes every P2P group a previous glint left behind and reports how
/// many went. Generic over the trait so the daemon's startup path is
/// testable without a radio.
pub async fn clean_stale_groups<L: P2pLink>(link: &L) -> Result<usize, LinkError> {
    let stale = link.stale_groups().await?;
    let count = stale.len();
    for id in stale {
        link.remove_group(id).await?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::fake::{FakeP2pLink, LinkCall};
    use crate::link::{GroupId, P2pLink};

    #[tokio::test]
    async fn cleaning_two_stale_groups_removes_both_and_reports_two() {
        // arrange
        let link =
            FakeP2pLink::new().with_stale_groups(vec![GroupId::new("g-1"), GroupId::new("g-2")]);
        // act
        let removed = clean_stale_groups(&link).await.unwrap();
        // assert
        assert_eq!(removed, 2);
        assert_eq!(
            link.calls(),
            vec![
                LinkCall::StaleGroups,
                LinkCall::RemoveGroup(GroupId::new("g-1")),
                LinkCall::RemoveGroup(GroupId::new("g-2")),
            ]
        );
        assert!(link.stale_groups().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cleaning_with_nothing_stale_removes_nothing() {
        // arrange
        let link = FakeP2pLink::new();
        // act
        let removed = clean_stale_groups(&link).await.unwrap();
        // assert
        assert_eq!(removed, 0);
        assert_eq!(link.calls(), vec![LinkCall::StaleGroups]);
    }
}
