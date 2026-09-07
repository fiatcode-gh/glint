//! Manual driver for Task 16: bring the P2P device up against one sink,
//! then take it back down.
//!
//! The TV must be on and in Miracast mode.
//!
//!     cargo run --example connect -- aa:bb:cc:dd:ee:ff

use std::time::Duration;

use glint::link::P2pLink;
use glint::link::network_manager::{DEVICE_STATE_ACTIVATED, NetworkManagerLink};
use glint::receiver::MacAddr;

/// How long to wait for the group to form after NetworkManager accepts the
/// activation. Generous on purpose: a stall here is the interesting result,
/// so the run must not report one before the negotiation has had a fair go.
const ACTIVATION_WINDOW: Duration = Duration::from_secs(45);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mac: MacAddr = std::env::args()
        .nth(1)
        .ok_or("pass the sink's MAC, as printed by the scan example")?
        .parse()?;

    let link = NetworkManagerLink::connect().await?;
    println!("target mac:        {mac}");
    println!();
    println!("--- this shares one radio with infrastructure Wi-Fi, so the ---");
    println!("--- machine's normal wireless connection may drop while it runs ---");
    println!();

    // Resolved against a fresh scan, and only against what `scan` returns:
    // a peer object expires from NetworkManager's list once a find lapses,
    // and a peer that advertises no WFD information elements is not a sink
    // this example has any business connecting to.
    println!("rescanning for about 10 s ...");
    let Some(peer) = link
        .scan()
        .await?
        .into_iter()
        .find(|found| found.mac == mac)
    else {
        println!("peer:              NOT FOUND");
        println!();
        println!("--- {mac} is not among the Wi-Fi Display sinks in range ---");
        println!("--- run the scan example: it lists what answered and what was dropped ---");
        return Ok(());
    };
    println!("peer name:         {}", peer.name);

    let handle = link.connect(&peer).await?;
    println!("activation:        accepted by NetworkManager");

    // The activation call returning proves only that NetworkManager took the
    // request. Group negotiation happens afterwards and can sit in `config`
    // indefinitely without error, so the device state is what decides
    // whether this gate passed.
    let mut state = link.device_state().await?;
    let mut waited = Duration::ZERO;
    let step = Duration::from_secs(3);
    while state != DEVICE_STATE_ACTIVATED && waited < ACTIVATION_WINDOW {
        tokio::time::sleep(step).await;
        waited += step;
        state = link.device_state().await?;
        println!(
            "  after {:>3}s:      device state {state}",
            waited.as_secs()
        );
    }

    if state == DEVICE_STATE_ACTIVATED {
        println!("link up:           yes, device state {state}");
    } else {
        println!(
            "link up:           NO - stalled at device state {state} after {}s",
            waited.as_secs()
        );
        println!("                   (100 is activated; 50 is `config`, still negotiating)");
    }

    link.disconnect(handle).await?;
    println!("deactivated:       yes");
    println!("final state:       {}", link.device_state().await?);
    println!();
    println!("--- `nmcli device` must show the wifi-p2p device disconnected ---");
    println!("--- `nmcli connection show` must list no `glint p2p ...` leftover ---");
    println!("--- this gate PASSES only if `link up` above said yes ---");
    Ok(())
}
