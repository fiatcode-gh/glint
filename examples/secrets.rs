//! Manual driver for the Secret Service store: set, read back, delete.
//!
//! Task 13 has no automated test. Run this with the wallet visible and watch
//! the entry appear and vanish in the wallet's own interface — that half is
//! what no test in this crate can assert.

use glint::receiver::MacAddr;
use glint::secrets::SecretStore;
use glint::secrets::secret_service::SecretServiceStore;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mac: MacAddr = "aa:bb:cc:dd:ee:ff".parse()?;
    let store = SecretServiceStore::connect().await?;

    println!("before set:   {:?}", store.get(mac).await?);

    store.set(mac, "hunter2").await?;
    println!("after set:    {:?}", store.get(mac).await?);
    println!("--- look for \"glint pairing {mac}\" in the wallet, then press enter ---");
    std::io::stdin().read_line(&mut String::new())?;

    store.delete(mac).await?;
    println!("after delete: {:?}", store.get(mac).await?);
    println!("--- the entry should now be gone from the wallet ---");

    Ok(())
}
