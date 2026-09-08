//! Manual driver for Tasks 24 and 25: the whole cast, link to pixels.
//!
//! # THIS CANNOT RUN YET
//!
//! It is compiled and reviewed, never executed. Wi-Fi Direct group formation
//! fails below NetworkManager (follow-up 9), so no sink can reach the RTSP
//! listener and the handshake this example drives cannot start. Everything it
//! needs is written down here so the gates can be run later without deriving
//! any of it again.
//!
//!     cargo run --example cast -- aa:bb:cc:dd:ee:ff
//!     cargo run --example cast -- aa:bb:cc:dd:ee:ff <restore token>
//!
//! The MAC is the sink's, as printed by `cargo run --example scan`. The
//! optional second argument is a portal restore token from
//! `cargo run --example capture`, which skips the picker.
//!
//! # The two deferred gates, and what to watch for
//!
//! Gate one — the CU7000 reaches PLAY. Watch the printed lines in order:
//! `link up`, then `sink connected`, then `negotiated`, then `PLAY`. If it
//! stops at `link up` the blocker is still follow-up 9. If a sink connects but
//! the handshake dies, the `flow failed:` line names the reason — and when the
//! television's bytes are what rtsp-types rejected, that same line carries
//! them escaped, reading `the peer sent bytes that are not RTSP: ...`. That
//! dump is the only instrument for a byte stream nobody has seen, and it is
//! what decides whether a tolerance shim is ever needed. Do not write one
//! before that dump says so.
//!
//! Gate two — the picture appears. With `PLAY` printed and the stats lines
//! showing a non-zero `bytes/s`, look at the television: that is
//! glass-to-glass. Time it with a stopwatch against a moving window on the
//! desktop for the latency figure.
//!
//! It also prints the RAW M3 reply body verbatim, under `raw M3 reply body`.
//! That text is the capture `tests/fixtures/README.md` is waiting for.
//!
//! Two things to expect on this host: infrastructure Wi-Fi may drop, because
//! `wlp1s0` and `p2p-dev-wlp1s0` share one radio; and the television must be
//! on, with Miracast or Screen Mirroring open.
//!
//! The crate logs through `tracing` and the workspace installs no subscriber,
//! so the flow's own warnings — the video-only fallback, an M3-versus-SETUP
//! port mismatch, a stream that will not parse — go nowhere here. Every
//! decision these two gates turn on is printed below instead, so nothing is
//! lost; the warnings only add detail, and reaching them needs a subscriber
//! this example deliberately does not pull in as a dependency.

use std::os::fd::AsRawFd;
use std::time::Duration;

use glint::capture::{ScreenSource, portal::PortalScreenSource};
use glint::link::P2pLink;
use glint::link::network_manager::NetworkManagerLink;
use glint::pipeline::build::build;
use glint::pipeline::encoder::detect_installed;
use glint::pipeline::runner::Runner;
use glint::pipeline::{Output, PipelineSpec};
use glint::receiver::MacAddr;
use glint::wfd::flow::FlowEvent;
use glint::wfd::rtsp::{RTSP_PORT, RtspListener};
use tokio::sync::mpsc;

/// GND's field cap. `config`'s `bitrate_cap_kbps` defaults to `None`, so
/// nothing in the tree decides this — and 20 Mbps of UDP over a radio already
/// shared with infrastructure Wi-Fi is the worse guess for a live cast.
const BITRATE_KBPS: u32 = 4096;

/// How long to wait for a sink to dial in and reach PLAY once the link is up.
/// Generous on purpose: a stall is the interesting result here, so the run
/// must not give up before the television has had a fair go.
const CAST_WINDOW: Duration = Duration::from_secs(60);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let mac: MacAddr = arguments
        .next()
        .ok_or("pass the sink's MAC, as printed by the scan example")?
        .parse()?;
    let restore_token = arguments.next();

    let encoder = detect_installed(None)?;
    println!("encoder:           {}", encoder.element());
    println!("target mac:        {mac}");
    println!();
    println!("--- this shares one radio with infrastructure Wi-Fi, so the ---");
    println!("--- machine's normal wireless connection may drop while it runs ---");
    println!();

    // The listener goes up BEFORE the link: the sink dials 7236 as soon as the
    // group forms, and a sink that finds nothing listening gives up.
    let listener = RtspListener::bind(format!("0.0.0.0:{RTSP_PORT}").parse()?).await?;
    println!("listening on:      {}", listener.local_addr()?);
    let (events_out, mut events) = mpsc::channel(32);
    tokio::spawn(async move {
        if let Err(error) = listener.serve(events_out).await {
            eprintln!("listener stopped:  {error}");
        }
    });

    let link = NetworkManagerLink::connect().await?;
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

    let mut capture = None;
    let mut format = None;
    let mut runner: Option<Runner> = None;

    // One pass over the flow's events. The portal is opened only once a sink
    // has actually negotiated, so a run blocked by follow-up 9 never puts a
    // screen picker in front of the user for nothing.
    loop {
        let event = match tokio::time::timeout(CAST_WINDOW, events.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                println!("flow:              the listener stopped");
                break;
            }
            Err(_) => {
                println!(
                    "PLAY:              NOT REACHED within {}s",
                    CAST_WINDOW.as_secs()
                );
                println!("                   no sink completed the handshake — with follow-up 9");
                println!("                   open, no group forms, so none can even connect");
                break;
            }
        };

        match event {
            FlowEvent::M3Captured(raw) => {
                // Verbatim, deliberately: this text is the fixture, and
                // normalising it would destroy the very bytes being recorded.
                println!("sink connected:    it answered M3");
                println!("raw M3 reply body: ---8<--- (verbatim, this is the fixture) ---");
                print!("{}", String::from_utf8_lossy(&raw));
                println!("--->8--- end of raw M3 reply body ---");
            }
            FlowEvent::Negotiated {
                width,
                height,
                fps,
                audio,
            } => {
                println!("negotiated:        {width}x{height}p{fps}, audio {audio}");
                format = Some((width, height, fps, audio));
            }
            FlowEvent::Play { rtp_host, rtp_port } => {
                println!("PLAY:              sending RTP to {rtp_host}:{rtp_port}");
                let Some((width, height, fps, audio)) = format else {
                    println!("                   but nothing was negotiated; not starting");
                    break;
                };

                let mut source = PortalScreenSource::new();
                let started = source.start(restore_token.as_deref()).await?;
                println!("node id:           {}", started.node_id);
                println!("restore token out: {:?}", started.restore_token);

                let description = build(&PipelineSpec {
                    encoder,
                    width,
                    height,
                    fps,
                    bitrate_kbps: BITRATE_KBPS,
                    audio,
                    video_node: started.node_id,
                    video_fd: started.fd.as_raw_fd(),
                    output: Output::Rtp {
                        host: rtp_host,
                        port: rtp_port,
                    },
                });
                println!("pipeline:          {description}");
                runner = Some(Runner::start(&description)?);
                // The pipeline holds only the descriptor NUMBER, so the
                // descriptor itself has to outlive it.
                capture = Some(started);
                println!();
                println!("--- LOOK AT THE TELEVISION: the screen should be there ---");
                println!();
                break;
            }
            FlowEvent::Teardown => {
                println!("flow:              the sink tore the session down");
                break;
            }
            FlowEvent::Failed(reason) => {
                println!("flow failed:       {reason}");
                break;
            }
        }
    }

    if let Some(runner) = runner {
        for second in 1..=10 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let stats = runner.sample();
            println!(
                "t+{second}s  bytes/s {:>9}  dropped {} (total)",
                stats.bytes_per_second, stats.dropped_frames
            );
        }
        runner.stop()?;
        println!();
        println!("--- non-zero bytes/s above AND a picture on the TV is the gate ---");
        println!("--- time a moving window with a stopwatch for glass-to-glass ---");
    }

    drop(capture);
    link.disconnect(handle).await?;
    println!("deactivated:       yes");
    println!();
    println!("--- `nmcli device` must show the wifi-p2p device disconnected ---");
    println!("--- `nmcli connection show` must list no `glint p2p ...` leftover ---");
    Ok(())
}
