# R4: VA-API H.264 encoder on the Radeon 780M

Status: **codec install approved, pending** — property measurements resume after
the install. Checked 2026-09-04, Fedora Kinoite 44, GStreamer 1.28.6.

## Findings

- The GStreamer `va` plugin is present and detects the GPU: elements are named
  "… in AMD Radeon 780M Graphics".
- It exposes `vaav1enc` and `vaav1dec` (AV1), `vacompositor` — but **no `vah264enc`**.
- Reason: Fedora ships the Mesa VA drivers without H.264/H.265 encode or decode
  (patent policy). The full driver is `mesa-va-drivers-freeworld` from RPM Fusion.
- The software fallback `x264enc` is also absent. Only `gstreamer1-plugins-ugly-free`
  is installed; `x264enc` lives in the non-free `gstreamer1-plugins-ugly` (RPM Fusion).
- Wi-Fi Display mandates H.264, and AV1 is not part of the WFD spec, so **no cast is
  possible on this machine until one of the two encoders is installed**.

## Three ways to get an H.264 encoder on Fedora

Checked 2026-09-04. All three are layered with `rpm-ostree` plus a reboot on
Kinoite (plain `dnf` on regular Fedora):

1. **`mesa-va-drivers-freeworld`** (RPM Fusion) — enables hardware encode on
   the 780M; GStreamer then exposes `vah264enc`. Best quality per watt. The
   only hardware option on Fedora.
2. **`gstreamer1-plugins-ugly`** (RPM Fusion) — the `x264enc` software
   encoder. High quality, heavier on the CPU.
3. **`gstreamer1-plugin-openh264` + `openh264`** — from the
   `fedora-cisco-openh264` repository, which is **enabled by default** on this
   machine (verified in `/etc/yum.repos.d/`). No third-party repository
   needed; Cisco pays the H.264 patent fees. The `openh264enc` element encodes
   Constrained Baseline profile only — which is exactly the profile the Wi-Fi
   Display specification requires every sink to support, so it is enough to
   cast, just with software-encode CPU cost.

Plan for the development machine (decided 2026-09-04): the full RPM Fusion
layering (options 1 and 2), because hardware encode is the primary path and
this file must record the `vah264enc` property names.

## What this means for distributing the app

The app never ships an encoder. It picks from what GStreamer offers at
runtime, so packaging is unaffected by the patent issue:

- Most distributions (Arch, Debian, Ubuntu) ship Mesa with the H.264 codecs
  enabled and package `x264enc`. Fedora and openSUSE are the outliers.
- The pipeline builder therefore uses a fallback chain —
  `vah264enc` → `x264enc` → `openh264enc` — and reports a clear error naming
  the packages to install when none is present. GNOME Network Displays handles
  the same situation the same way.
- A future Flatpak removes the problem even on stock Fedora: the Flathub
  runtime's Mesa is built with the codecs enabled, the
  `org.freedesktop.Platform.openh264` extension installs automatically, and
  bundling x264 is permitted on Flathub (OBS Studio does).

## Still to measure (after the codec install)

Run `gst-inspect-1.0` on `vah264enc`, `x264enc`, and `openh264enc` and record
here which of these properties each exposes, with their exact names:

- rate control mode (constant bitrate)
- B-frame count (must be forceable to zero)
- keyframe / IDR interval

These names feed Task 10 (pipeline string builder) in the implementation plan.

#miracast #research
