# R4: VA-API H.264 encoder on the Radeon 780M

Status: **blocked on codec install** — checked 2026-09-04, Fedora Kinoite 44, GStreamer 1.28.6.

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

## To unblock

On Kinoite this means enabling the RPM Fusion repositories and layering packages
with `rpm-ostree` (replace `mesa-va-drivers` with `mesa-va-drivers-freeworld`,
add `gstreamer1-plugins-ugly`), followed by a reboot. Needs the user's go-ahead.

## Still to measure (after unblock)

Re-run `gst-inspect-1.0 vah264enc` and record here which of these properties the
driver exposes, with their exact names:

- rate control mode (constant bitrate)
- B-frame count (must be forceable to zero)
- keyframe / IDR interval

These names feed Task 10 (pipeline string builder) in the implementation plan.

#miracast #research
