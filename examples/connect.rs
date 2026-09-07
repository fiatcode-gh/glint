//! Manual driver for Task 16: bring the P2P device up against one sink,
//! then take it back down.
//!
//! The TV must be on and in Miracast mode. `wlp1s0` and `p2p-dev-wlp1s0`
//! share one radio, so infrastructure Wi-Fi may drop while this runs.
//!
//!     cargo run --example connect -- aa:bb:cc:dd:ee:ff

use std::time::Duration;

use glint::link::network_manager::NetworkManagerLink;
use glint::link::{P2pLink, Peer};
use glint::receiver::MacAddr;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mac: MacAddr = std::env::args()
        .nth(1)
        .ok_or("pass the sink's MAC, as printed by the scan example")?
        .parse()?;

    let link = NetworkManagerLink::connect().await?;
    println!("target mac:        {mac}");

    // A peer object expires from NetworkManager's list once a find lapses,
    // so the MAC is resolved against a fresh scan rather than assumed to
    // still be visible.
    println!("rescanning for about 10 s ...");
    let peer = link
        .scan()
        .await?
        .into_iter()
        .find(|found| found.mac == mac)
        .unwrap_or(Peer {
            mac,
            name: String::new(),
        });
    println!("peer name:         {}", peer.name);

    let handle = link.connect(&peer).await?;
    println!("connected:         yes");
    println!();
    println!("--- run `nmcli device` NOW: p2p-dev-wlp1s0 must read connected ---");
    println!("--- disconnecting in 30 s ---");
    tokio::time::sleep(Duration::from_secs(30)).await;

    link.disconnect(handle).await?;
    println!("disconnected:      yes");
    println!();
    println!("--- run `nmcli device` again: p2p-dev-wlp1s0 must read disconnected ---");
    println!("--- and `nmcli connection show` must list no `glint p2p ...` leftover ---");
    Ok(())
}
