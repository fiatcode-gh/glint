//! `glintd` entry point. Milestone 1 ships a version stub; the real daemon
//! (D-Bus service plus session loop) arrives in Milestone 3.

fn main() {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
}
