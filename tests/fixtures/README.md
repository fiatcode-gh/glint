PLACEHOLDER: `m3_reply_video_formats.txt` is not a real capture — it is a byte-identical copy of the canonical value in `tests/wfd_params.rs`, so the test that reads it proves nothing.

Replacing it needs a real television's M3 reply, which is blocked on follow-up 9: Wi-Fi Direct group formation fails below NetworkManager, so no sink can reach glint's RTSP listener to send one.

The instrument is `cargo run --example cast -- <sink MAC>`, which prints the raw M3 reply body verbatim under `raw M3 reply body`. When that capture happens, the verbatim text lands in a sidecar `m3_reply_raw.txt` and the normalised `wfd_video_formats` value replaces this file. No empty sidecar is committed in the meantime: a committed empty fixture reads to a later reader as though a capture had already happened.
