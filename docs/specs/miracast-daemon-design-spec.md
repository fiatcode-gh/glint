# Design Spec: Desktop-Independent Linux Miracast Sender

Status: approved design, pre-implementation
Name: **glint** (daemon: `glintd`, command-line client: `glintctl`, D-Bus name: `dev.fiatcode.Glint`, config: `~/.config/glint/`)
Author: Dhemas (fiatcode)
Date: 2026-09-03

---

## 1. Goal

A Linux app that casts the screen to a Miracast receiver (smart TV or Windows PC) with the reliability and feature set of Windows "Connect" / "Wireless Display". It must work on any desktop, not only GNOME.

Version one closes four gaps that GNOME Network Displays (GND) leaves open:

1. Works on KDE, Wayland, and other desktops, not only GNOME.
2. Extend mode: the receiver becomes a second virtual monitor.
3. Reliability: remembered pairing, fast reconnect, automatic retry.
4. Audio that stays in sync, with low latency.

Direction for version one: **send only**. Receiving (acting as a wireless display) is out of scope.
Protocol for version one: **Miracast over Wi-Fi Direct** only. Chromecast, AirPlay, DLNA, and Miracast over Infrastructure are out of scope.

---

## 2. Architecture: daemon plus thin clients

Two processes, one contract between them.

- **Daemon**: headless, runs as a user systemd service, owns all protocol and media work.
- **Clients**: any process that speaks the daemon's D-Bus interface. First client is a Flutter desktop app. A command-line client ships alongside it.

Reason: desktop independence and Extend mode are structural. Reconnect logic lives in one place and survives UI restarts. A UI crash never drops the stream.

---

## 3. User-facing behavior

- Tray or panel icon plus a small window. Opening the window starts a scan and lists nearby Miracast receivers by friendly name. Previously paired receivers carry a "known" marker.
- Selecting a receiver offers two modes: **Mirror** and **Extend**. Extend is offered only if the current compositor backend supports virtual outputs. Otherwise it is greyed out with a one-line reason.
- Pairing follows the receiver's method: PIN shown on the TV, or push-button. PINs are stored, so the second connection to the same receiver is one tap.
- While connected the window shows resolution, bitrate, and a latency estimate. Controls: switch mode, mute audio, disconnect.
- Audio follows the screen by default through PipeWire. A toggle keeps audio local.
- On link loss the daemon retries a known receiver for a configurable time before giving up. The UI shows "retrying" rather than silently ending.
- Closing the window does not end the cast. The tray icon stays.

---

## 4. Technical decisions

| Concern | Decision | Reason |
|---|---|---|
| Link layer | NetworkManager Wi-Fi P2P device API (`NMDeviceWifiP2P`, NetworkManager 1.16+). The P2P connection must advertise WFD information elements (`wifi-p2p.wfd-ies`) so sinks recognize us as a Miracast source, not a plain Wi-Fi Direct peer | Same path GND uses; works on Fedora and most distros without extra setup. No direct wpa_supplicant control. |
| Protocol | Wi-Fi Display (WFD) as specified. Source runs an RTSP listener on TCP 7236; receiver connects; MPEG-TS over RTP/UDP | Standard behavior. The RTSP control channel is hand-rolled (plain TCP listener plus the `rtsp-types` crate): WFD reverses the RTSP roles, and the `gst-rtsp-server` Rust bindings lack the hooks needed to bend it (see `docs/research/rtsp-approach.md`). The media pipeline stays in GStreamer either way. |
| Screen capture | xdg-desktop-portal ScreenCast interface, yielding a PipeWire stream | One code path for KDE, GNOME, and wlroots compositors. |
| Audio capture | PipeWire monitor of the default sink | Consistent with video capture; same clock domain available. |
| Encode | GStreamer pipeline: PipeWire source, H.264 encoder, MPEG-TS mux with audio, RTP payload, UDP sink | Encoder chosen at runtime down a fallback chain: `vah264enc` (hardware VA-API), `x264enc`, `openh264enc`; a clear error names the packages to install when none is present. The app never ships an encoder, so distributions without H.264 (stock Fedora, openSUSE) still work after one package install — or via a future Flatpak, whose runtime carries the codecs. |
| Extend mode | xdg-desktop-portal ScreenCast with source type VIRTUAL (bit 4); runtime probe of `AvailableSourceTypes` greys out Extend when the bit is absent | One cross-desktop code path, researched 2026-09-04: works on KDE (Plasma 5.25+, live-verified on 6.7.4) and GNOME (42+). Compositor-specific fallbacks exist if resolution control is ever needed (KWin `zkde_screencast_unstable_v1`, Mutter `RecordVirtual`) — see `docs/research/kwin-virtual-output.md` and `docs/research/mutter-virtual-output.md`. wlroots: unverified. |
| Control surface | One D-Bus service on the session bus. Objects: daemon, each discovered receiver, active session. Signals on every state change | Clients never poll. Any D-Bus-capable tool can drive it. |
| Daemon language | Rust, using `gstreamer-rs` and `zbus` | Memory safety in a long-running network daemon, first-class GStreamer bindings, D-Bus without GLib. |
| First client | Flutter desktop, talking D-Bus | Matches author's stack. Qt/Kirigami later if KDE integration demands it. |

Dart is not used in the daemon because it has no usable GStreamer bindings.

---

## 5. Data and state

### Persistent, on disk (`~/.config/<appname>/`, TOML)

- Known receivers: MAC address, friendly name, last-used mode, last-used resolution, saved portal restore token.
- User settings: preferred encoder, bitrate cap, audio-follows-screen default, retry timeout.

### Persistent, in Secret Service (KDE Wallet or GNOME Keyring via the same D-Bus interface)

- Pairing secrets: PINs, WPS credentials. Never written to TOML.

### Runtime only, in daemon memory

- Current scan results with signal strength. Refreshed per scan, dropped on daemon stop.
- Active session: receiver, mode, negotiated codec and resolution, live stats (bitrate, dropped frames, latency estimate). Exposed over D-Bus, never written.
- Connection state machine:
  `Idle -> Scanning -> Connecting -> Pairing -> Negotiating -> Streaming -> Reconnecting -> Idle`
  Every transition emits a D-Bus signal.

### Logs

- systemd journal only, structured. No own log files.
- Debug flag adds RTSP message dumps. Off by default.

### Privacy

- Nothing leaves the machine except the stream. No telemetry, no cloud.
- Known-receivers TOML is human-readable and portable. Secrets are not exported.

---

## 6. Edge cases and constraints

| Issue | Handling |
|---|---|
| Concurrent P2P and infrastructure Wi-Fi | Chip and driver dependent. Daemon detects loss of the infrastructure connection during casting and warns the user. |
| Driver P2P support unverified on the development laptop | **Blocker to check before any code.** `iw list` must show `P2P-client` and `P2P-GO` under supported interface modes. If missing, use a USB adapter with a known-good chipset. |
| Extend on KDE may not be possible | If KWin exposes no reachable virtual output API, Extend ships greyed out on KDE. Not promised in the README until confirmed. |
| Windows receivers prefer Miracast over Infrastructure (MS-MICE) | Out of scope for version one. Documented follow-up. Needs mDNS discovery and a separate code path. |
| HDCP (content protection) | Cannot be supported on Linux. Daemon surfaces a clear error when a receiver refuses for this reason. |
| Encoder latency | Force low-latency tune: short GOP, no B-frames, constant bitrate. Latency estimate is derived unless the receiver echoes timestamps. |
| Portal permission prompt on every connect | Save the portal restore token with the known receiver. |
| Audio and video drift | Mux runs on a single pipeline clock. |
| Daemon crash mid-stream | systemd restarts it. On startup, daemon finds and tears down stale P2P connections it owns. |
| Receiver quirks (Samsung, LG, cheap Android TVs) | Per-vendor quirks table from day one, keyed on WFD device info. |

---

## 7. Out of scope for version one

- Receiving (sink mode).
- Chromecast / Google Cast, AirPlay, DLNA/UPnP.
- Miracast over Infrastructure.
- HDCP.
- Extend mode on compositors without a verified virtual output backend.

---

## 8. Open research items (resolve before implementation planning is final)

1. ~~Does the development laptop's Wi-Fi chip support P2P-client and P2P-GO modes?~~ **Yes** (verified 2026-09-04: `P2P-client` and `P2P-GO` in `iw list`, NetworkManager 1.56.1).
2. ~~What, if anything, does KWin expose for creating a virtual output that an external process can drive?~~ **Answered** — portal ScreenCast VIRTUAL type, plus KWin's gated `zkde_screencast_unstable_v1`; see `docs/research/kwin-virtual-output.md`.
3. ~~Exact Mutter D-Bus calls used by gnome-remote-desktop for virtual monitors, and their stability across GNOME versions.~~ **Answered** — `RecordVirtual` on `org.gnome.Mutter.ScreenCast.Session`; private API, so the portal VIRTUAL type is the recommended route; see `docs/research/mutter-virtual-output.md`.
4. Which VA-API H.264 low-latency parameters the Radeon 780M driver honors — **pending a codec install**; the encoder landscape and fallback chain are documented in `docs/research/vaapi-encoder.md`.
5. ~~RTSP approach.~~ **Decided: hand-roll.** The gst-rtsp-server Rust bindings lack the hooks Wi-Fi Display needs (no `send_message` vfunc, no session-pool subclassing); glint uses a plain TCP listener on 7236 with the `rtsp-types` crate; see `docs/research/rtsp-approach.md`.

#miracast #linux #rust #flutter #design-spec
