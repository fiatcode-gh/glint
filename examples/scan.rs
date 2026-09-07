//! Manual driver for Task 15: list the Wi-Fi Display sinks in range.
//!
//! The TV must be on and in Miracast mode before this runs. It takes about
//! ten seconds: that is the scan window, not a hang.
//!
//!     cargo run --example scan

use glint::link::P2pLink;
use glint::link::network_manager::NetworkManagerLink;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let link = NetworkManagerLink::connect().await?;
    println!("scanning for about 10 s ...");
    let peers = link.scan().await?;

    println!("sinks found:       {}", peers.len());
    for peer in &peers {
        println!("  name:            {}", peer.name);
        println!("  mac:             {}", peer.mac);
    }
    println!();
    println!("--- the TV's own name and MAC must appear above ---");
    println!("--- an empty list means either the TV is not in Miracast mode, or it ---");
    println!("--- advertises no WFD information elements and was filtered out ---");
    Ok(())
}
