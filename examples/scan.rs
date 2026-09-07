//! Manual driver for Task 15: list the Wi-Fi Display sinks in range.
//!
//! The TV must be on and in Miracast mode before this runs. It takes about
//! ten seconds: that is the scan window, not a hang.
//!
//! It prints twice over one window. First the sinks `scan` kept, which is
//! the gate. Then every peer NetworkManager advertised with its raw WFD
//! information elements in hex, which is how a sink that answered is told
//! apart from one dropped for advertising none.
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
    let advertised = link.advertised_peers().await?;
    println!("peers advertised:  {}", advertised.len());
    for peer in &advertised {
        let hex: Vec<String> = peer.wfd_ies.iter().map(|b| format!("{b:02x}")).collect();
        println!("  name:            {}", peer.name);
        println!("  hwaddress:       {}", peer.hw_address);
        println!("  wfd-ies:         {}", hex.join(" "));
    }

    println!();
    println!("--- the TV's own name and MAC must appear under `sinks found` ---");
    println!("--- an empty list means either the TV is not in Miracast mode, or it ---");
    println!("--- appears under `peers advertised` with empty wfd-ies and was dropped ---");
    Ok(())
}
