//! Manual driver for Task 23: bind the Wi-Fi Display RTSP port, accept one
//! peer at a time, and log every message it sends.
//!
//! No television needed — `nc` is the sink for this gate. It deliberately does
//! NOT run the WFD flow: this checks the transport, so nothing is ever sent
//! back and every reply the peer gets is silence.
//!
//!     cargo run --example listen
//!
//! Then, in another terminal. `nc` is NOT installed on the development host,
//! so `socat` is what the gate was actually run with; either sends the bytes:
//!
//!     ss -ltn | grep 7236
//!     printf 'OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nRequire: org.wfa.wfd1.0\r\n\r\n' | socat -t1 - TCP:127.0.0.1:7236
//!     printf 'this is not rtsp \001\002\r\n' | socat -t1 - TCP:127.0.0.1:7236
//!
//! The gate passes when `ss` shows something listening on 7236, the OPTIONS is
//! logged as a parsed request, and the garbage line is logged as escaped raw
//! bytes rather than crashing the listener.
//!
//! A garbage fragment with NO line ending logs nothing: rtsp-types calls that
//! `Incomplete`, correctly, because more bytes could still complete it. Press
//! Enter, or use `printf` as above.

use glint::wfd::rtsp::{FrameDecoder, RTSP_PORT};
use rtsp_types::Message;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

/// One read's worth of buffer; the decoder reassembles across reads.
const READ_CHUNK: usize = 4096;

fn describe(message: &Message<Vec<u8>>) -> String {
    match message {
        Message::Request(request) => format!(
            "request  {:?} {}",
            request.method(),
            request
                .request_uri()
                .map(|uri| uri.to_string())
                .unwrap_or_else(|| "*".to_string())
        ),
        Message::Response(response) => format!(
            "response {:?} {:?}",
            response.status(),
            response.reason_phrase()
        ),
        Message::Data(frame) => {
            format!(
                "data     channel {}, {} bytes",
                frame.channel_id(),
                frame.len()
            )
        }
    }
}

async fn log_one_peer(mut stream: TcpStream) {
    let mut decoder = FrameDecoder::default();
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = match stream.read(&mut buffer).await {
            Ok(0) => {
                println!("  peer hung up");
                return;
            }
            Ok(count) => count,
            Err(error) => {
                println!("  read failed:     {error}");
                return;
            }
        };
        println!("  read:            {read} bytes");
        decoder.push(&buffer[..read]);
        loop {
            match decoder.next() {
                Ok(Some(message)) => {
                    println!("  parsed:          {}", describe(&message));
                    let body = match &message {
                        Message::Request(request) => request.body().as_slice(),
                        Message::Response(response) => response.body().as_slice(),
                        Message::Data(_) => &[],
                    };
                    if !body.is_empty() {
                        println!(
                            "  body:            {}",
                            String::from_utf8_lossy(body).escape_debug()
                        );
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    // The whole point of the gate: unparsable bytes have to be
                    // legible in the log rather than lost or fatal.
                    println!("  PARSE ERROR:     {error}");
                    return;
                }
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = format!("0.0.0.0:{RTSP_PORT}");
    let listener = TcpListener::bind(&address).await?;
    println!("listening on:      {}", listener.local_addr()?);
    println!();
    println!("--- check `ss -ltn | grep {RTSP_PORT}` shows this socket ---");
    println!("--- send an OPTIONS with nc: it must be logged as a parsed request ---");
    println!("--- send a garbage LINE with nc: it must log escaped bytes, not crash ---");
    println!("--- ctrl-c to stop ---");
    println!();

    loop {
        let (stream, peer) = listener.accept().await?;
        println!("accepted:          {peer}");
        log_one_peer(stream).await;
        println!();
    }
}
