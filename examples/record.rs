//! Manual driver for Tasks 21 and 22: five seconds of screen and audio to a
//! `.ts` file, with one stats line per second.
//!
//! Nothing in the crate covers this path automatically — it needs a real
//! desktop, a real portal and a real sound server. Have something PLAYING to
//! the default sink while it runs, or the audio track will be silence and the
//! playback check proves nothing.
//!
//!     cargo run --example record
//!     cargo run --example record -- <a restore token from examples/capture>
//!
//! Then play it back and check that video and audio are both there, and in
//! sync:  mpv /tmp/glint-record.ts

use std::os::fd::AsRawFd;

use glint::capture::{ScreenSource, portal::PortalScreenSource};
use glint::pipeline::build::build;
use glint::pipeline::encoder::detect_installed;
use glint::pipeline::runner::Runner;
use glint::pipeline::{Output, PipelineSpec};

const SECONDS: u32 = 5;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut source = PortalScreenSource::new();
    let capture = source.start(std::env::args().nth(1).as_deref()).await?;
    println!("node id:           {}", capture.node_id);
    println!("restore token out: {:?}", capture.restore_token);

    let encoder = detect_installed(None)?;
    println!("encoder:           {}", encoder.element());

    let path = std::env::temp_dir().join("glint-record.ts");
    let description = build(&PipelineSpec {
        encoder,
        width: 1920,
        height: 1080,
        fps: 30,
        bitrate_kbps: 20_000,
        audio: true,
        video_node: capture.node_id,
        video_fd: capture.fd.as_raw_fd(),
        output: Output::File(path.display().to_string()),
    });
    println!("pipeline:          {description}");
    println!();

    let runner = Runner::start(&description)?;
    for second in 1..=SECONDS {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let stats = runner.sample();
        println!(
            "t+{second}s  bytes/s {:>9}  dropped {}",
            stats.bytes_per_second, stats.dropped_frames
        );
    }
    runner.stop()?;

    // The pipeline holds only the descriptor NUMBER, so the descriptor itself
    // has to outlive it. Dropping it here says that, rather than leaving the
    // lifetime to the accident of declaration order.
    drop(capture);

    println!();
    println!("wrote {}", path.display());
    println!("--- play it: mpv {} ---", path.display());
    println!("--- video AND audio, and in sync? that is the gate ---");
    Ok(())
}
