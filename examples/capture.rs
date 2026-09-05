//! Manual driver for Task 18: show the picker once, then reuse the token.
//!
//! Run it bare and the KDE picker appears. Run it again passing the token it
//! printed and no picker should appear at all. That second run is the only
//! thing that proves restore works, and no test in this crate can assert it.
//!
//!     cargo run --example capture
//!     cargo run --example capture -- <the token from the first run>

use glint::capture::{ScreenSource, portal::PortalScreenSource};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::args().nth(1);
    println!("restore token in:  {token:?}");

    let mut source = PortalScreenSource::new();
    let capture = source.start(token.as_deref()).await?;

    println!("node id:           {}", capture.node_id);
    println!("restore token out: {:?}", capture.restore_token);
    println!();
    println!("--- rerun with that token as the first argument; the picker must NOT appear ---");
    println!("--- a token is single-use: each run prints a fresh one to carry forward ---");
    Ok(())
}
