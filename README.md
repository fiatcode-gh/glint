# glint

Cast your Linux screen to a Miracast receiver — a smart TV or a Windows PC — on
any desktop, not only GNOME.

**Status: pre-implementation.** The approved design is in
[docs/specs/miracast-daemon-design-spec.md](docs/specs/miracast-daemon-design-spec.md).
Research notes land in [docs/research/](docs/research/).

## Shape

- `glintd` — a headless Rust daemon (user systemd service) that owns the
  Wi-Fi Direct link, the Wi-Fi Display (WFD/RTSP) protocol, and the GStreamer
  pipeline. Driven entirely over D-Bus (`dev.fiatcode.Glint`).
- `glintctl` — a command-line client.
- A Flutter desktop app as the graphical client.

Version one is send-only Miracast over Wi-Fi Direct: Mirror mode, audio through
PipeWire, remembered pairings, automatic reconnect. Extend mode (the receiver as
a second monitor) follows where the compositor supports virtual outputs.
