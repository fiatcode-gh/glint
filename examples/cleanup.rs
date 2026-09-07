//! Manual driver for Task 17: remove the P2P connections a previous run
//! left behind.
//!
//! Kill the connect example mid-connection, then run this.
//!
//!     cargo run --example cleanup

use glint::link::P2pLink;
use glint::link::network_manager::NetworkManagerLink;
use glint::startup;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let link = NetworkManagerLink::connect().await?;

    // Listed before the removal because `clean_stale_groups` reports only a
    // count, and the human gate has to see what actually went.
    let stale = link.stale_groups().await?;
    println!("stale groups:      {}", stale.len());
    for group in &stale {
        println!("  connection:      {}", group.as_str());
    }

    let removed = startup::clean_stale_groups(&link).await?;
    println!("removed:           {removed}");
    println!("still stale:       {}", link.stale_groups().await?.len());
    println!();
    println!("--- `nmcli connection show` must now list no `glint p2p ...` entry ---");
    println!("--- the paths above are NM connection objects; nmcli shows their names ---");
    Ok(())
}
