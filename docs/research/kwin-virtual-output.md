# R2: Can an external process make KWin create a virtual output?

Research note for glint, 2026-09-04. Test machine: Fedora Kinoite 44, KDE Plasma 6.7.4 (Wayland). The live D-Bus measurements were verified twice: by a research agent and independently re-run by the architect session.

## Verdict

Yes — two usable APIs exist, so the Extend option does not need to be greyed out on KDE. The clean path is the xdg-desktop-portal ScreenCast interface with source type VIRTUAL (value 4), which KDE's portal advertises on this machine today. The lower-level path is the `stream_virtual_output` request of KWin's `zkde_screencast_unstable_v1` Wayland protocol, available since Plasma 5.24, which additionally lets the app choose the output's resolution — but it is gated behind a desktop-file permission.

## Evidence

### 1. KWin's screencast protocol has a virtual-output request

The protocol XML `zkde-screencast-unstable-v1.xml` in the [plasma-wayland-protocols](https://invent.kde.org/libraries/plasma-wayland-protocols/-/blob/master/src/protocols/zkde-screencast-unstable-v1.xml) repository defines interface `zkde_screencast_unstable_v1` (currently version 6) with:

- `stream_virtual_output` (since version 2) — arguments: stream, name (string), width (int), height (int), scale (fixed), pointer mode (uint). Protocol commit "screencast: Extend the protocol to allow streaming virtual outputs", October 2021.
- `stream_virtual_output_with_description` (since version 4) — adds a human-readable description string. Commit dated November 2024, so roughly the Plasma 6.3 era.

KWin's implementation ([src/plugins/screencast/screencastmanager.cpp](https://invent.kde.org/plasma/kwin/-/blob/master/src/plugins/screencast/screencastmanager.cpp)) creates a real output, not just a video feed:

```cpp
auto output = kwinApp()->outputBackend()->createVirtualOutput(name, description, size, scale);
streamOutput(stream, workspace()->findOutput(output), mode);
```

The output joins the workspace like a plugged-in monitor, so the desktop extends onto it. The compositor-side support was committed to kwayland-server on 2021-10-22 ("screencast: Implement version 2 of the protocol"), eight days after the Plasma 5.23 release, so it first shipped in Plasma 5.24 (February 2022).

**Access is gated.** On this machine, `wayland-info` lists 354 globals and `zkde_screencast_unstable_v1` is not among them. KWin (branch Plasma/6.7, `src/wayland_server.cpp`) keeps the interface on a blacklist and only reveals it to clients whose installed desktop file declares it in an `X-KDE-Wayland-Interfaces` line, matched by the client's executable path (or by sandbox security context). Shipped example on this machine, `/usr/share/applications/org.kde.krfb.virtualmonitor.desktop`:

```
Exec=/usr/bin/krfb-virtualmonitor
X-KDE-Wayland-Interfaces=zkde_screencast_unstable_v1
```

KWin's master branch has since relaxed this: its `allowInterface` now only blocks sandboxed (Flatpak/Snap) clients. That relaxation is not in 6.7.4.

### 2. KRdp and krfb

- **krfb-virtualmonitor** is the proof this works end to end: a dedicated binary (shipped in the krfb package, here krfb 26.08.0) that asks KWin for a virtual-output stream over `zkde_screencast_unstable_v1` and serves it over VNC (Virtual Network Computing). Introduced by Aleix Pol in [krfb commit aa12743e](https://invent.kde.org/network/krfb/-/commit/aa12743ea11a808b42e4e697ed81389403ee09ec), 2021-10-14. It is the best reference implementation for glint's direct-protocol path.
- **KRdp** (KDE's RDP — Remote Desktop Protocol — server) captures through the FreeDesktop Remote Desktop portal instead; its README (`/usr/share/doc/krdp/README.md`) says it "will open a remote desktop session on startup and reuse that session for all RDP connections". It streams existing monitors and does not itself create virtual outputs.

### 3. Other KWin mechanisms

- `kwin_wayland --help` on this machine shows `--virtual` ("Render to a virtual framebuffer"), but that selects a headless backend at compositor startup — not usable against a running session.
- KWin's D-Bus (Desktop Bus) service `org.kde.KWin` was introspected at `/KWin`: no output-creation method there. No D-Bus API for creating outputs was found; the Wayland protocol and the portal are the mechanisms.

### 4. The portal path: source type VIRTUAL

The [xdg-desktop-portal ScreenCast specification](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html) defines `AvailableSourceTypes` as a bitmask: "1: MONITOR: Share existing monitors, 2: WINDOW: Share application windows, 4: VIRTUAL: Extend with new virtual monitor". This is the same cross-desktop mechanism GNOME uses for virtual monitors, so one portal code path can cover both desktops.

KDE's portal backend gained it in May 2022 (xdg-desktop-portal-kde commit "wayland: Support virtual outputs when screensharing", Plasma 5.25 development cycle), initially with a fixed Full HD (1920x1080) resolution. A user-configurable size was only added in August 2026 ("Allow the user to set a custom size for virtual screens"). The portal never lets the requesting app pick the size.

Live check on this machine (Plasma 6.7.4), read over D-Bus with `busctl`:

- `org.freedesktop.impl.portal.ScreenCast AvailableSourceTypes` = `u 7` (MONITOR + WINDOW + VIRTUAL)
- `org.freedesktop.portal.ScreenCast AvailableSourceTypes` = `u 7`, interface `version` = `u 5`

So a plain portal client that passes `types = 4` to SelectSources gets a virtual monitor today, with the user approving in the normal portal dialog.

### 5. Local machine summary

`rpm -q`: kwin 6.7.4-2.fc44, plasma-workspace 6.7.4-2.fc44, xdg-desktop-portal-kde 6.7.4-1.fc44, krdp 6.7.4-1.fc44, krfb 26.08.0-1.fc44 (includes `/usr/bin/krfb-virtualmonitor`). The plasma-wayland-protocols package is not installed and no protocol XML sits under `/usr/share` — it is a build-time dependency, so glint must vendor the XML.

## Implications for glint's Extend mode

- **Prefer the portal path.** Request source type VIRTUAL (4) in the ScreenCast portal's SelectSources call. It needs no special privileges, works from a sandbox, reuses the existing portal plumbing, and works on GNOME too (see the sibling Mutter research, which reached the same conclusion independently).
- **The direct protocol is the resolution-exact fallback.** Speaking `zkde_screencast_unstable_v1` (vendored XML, since version 2) lets glint create the output at exactly the TV's native mode. It requires installing a desktop file with `X-KDE-Wayland-Interfaces=zkde_screencast_unstable_v1` whose Exec matches the daemon binary, and it is KDE-only.
- **Capability probe at runtime:** read `AvailableSourceTypes` from the portal; grey out Extend only when bit 4 is absent (very old Plasma, or non-supporting compositors).

## What this research did NOT check

- No virtual output was actually created — all checks were read-only. The portal VIRTUAL flow was not exercised end to end (dialog behavior, PipeWire stream properties, cursor mode, what happens on stream close).
- How the virtual output interacts with KScreen layout (position, scale persistence) and whether Plasma restores it across sessions.
- Exact Plasma minor release that first shipped the portal's VIRTUAL support (the commit is May 2022; 5.25 versus 5.26 was not confirmed against release tags).
- Behavior when glint runs inside a Flatpak (KWin blocks the direct protocol for sandboxed clients even on master; the portal path should still work but was not tested sandboxed).
- The GNOME/Mutter side (RecordVirtual, gnome-remote-desktop) — covered by the sibling research note `mutter-virtual-output.md`.
- Audio, damage/frame-rate characteristics of virtual-output streams, and multi-virtual-output limits.

#miracast #research
