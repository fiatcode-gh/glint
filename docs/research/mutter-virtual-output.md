# R3: Mutter virtual monitor path

## VERDICT

For a future GNOME "Extend screen" mode, glint should use the `xdg-desktop-portal` ScreenCast portal's `VIRTUAL` source type, not Mutter's private `org.gnome.Mutter.ScreenCast.RecordVirtual` call — GNOME's own portal backend (`xdg-desktop-portal-gnome`) implements `VIRTUAL` by calling `RecordVirtual` internally, so the portal route gets the same feature on a documented, stable-ish contract instead of an interface GNOME explicitly reserves the right to break.

Researched 2026-09-04. Key interface claims were verified twice: by a research agent reading the repositories, and independently by fetching the raw interface XML files.

## Versions checked

- Mutter, git tag `50.2` (part of the GNOME 50 stable series; as of 2026-09-04 GNOME 50.x is the current stable series, GNOME 51 is still in `.rc`).
- gnome-remote-desktop, git tag `50.2`.
- xdg-desktop-portal, `main` branch (the portal spec XML is not version-tagged per GNOME release; content read 2026-09-04).
- xdg-desktop-portal-gnome, `main` branch (content read 2026-09-04).

## Mutter's `org.gnome.Mutter.ScreenCast` D-Bus API

Source: `data/dbus-interfaces/org.gnome.Mutter.ScreenCast.xml` in the mutter repo at tag `50.2` (https://gitlab.gnome.org/GNOME/mutter/-/blob/50.2/data/dbus-interfaces/org.gnome.Mutter.ScreenCast.xml).

- Bus name and object path (from `src/backends/meta-screen-cast.c` at tag `50.2`, https://gitlab.gnome.org/GNOME/mutter/-/blob/50.2/src/backends/meta-screen-cast.c):
  `#define META_SCREEN_CAST_DBUS_SERVICE "org.gnome.Mutter.ScreenCast"`
  `#define META_SCREEN_CAST_DBUS_PATH "/org/gnome/Mutter/ScreenCast"`
  This is a session-bus service exported by mutter itself, not a separate daemon.
- Interface `org.gnome.Mutter.ScreenCast`: method `CreateSession(a{sv} properties) -> (o session_path)`. Properties include `"remote-desktop-session-id" (s)` (link to a companion RemoteDesktop session) and `"disable-animations" (b)`.
- Interface `org.gnome.Mutter.ScreenCast.Session` (on the returned `session_path`): methods `Start()`, `Stop()`; signal `Closed()`; and four "record" methods: `RecordMonitor(s connector, a{sv} properties) -> (o stream_path)`, `RecordWindow(a{sv} properties) -> (o stream_path)`, `RecordArea(i x, i y, i width, i height, a{sv} properties) -> (o stream_path)`, and the one glint would need for extend mode:
  `RecordVirtual(a{sv} properties) -> (o stream_path)`
  Documented properties for `RecordVirtual`:
  - `"cursor-mode" (u)`: 0 = hidden, 1 = embedded in the frame, 2 = sent as PipeWire stream metadata. Default hidden.
  - `"is-platform" (b)`: default `FALSE`. When `TRUE`, the virtual output "will not be interpreted as if the screen is shared, but more transparently as if it was a real monitor" — this is the extend-mode flag.
  - `"modes" (aa{sv})`: optional fixed mode list, each entry `{"size": (uu), "refresh-rate": d, "is-preferred": b}`; exactly one mode must be marked preferred. If set, the stream becomes non-resizable and behaves like a real monitor mode list.
- Interface `org.gnome.Mutter.ScreenCast.Stream` (on the returned `stream_path`): methods `Start()`, `Stop()`; signal `PipeWireStreamAdded(u node_id)`; property `Parameters (a{sv})` carrying `"position" (ii)` and `"size" (ii)` in compositor coordinates.
- The XML's own doc comment on the interface: "This API is private and not intended to be used outside of the integrated system that uses libmutter. No compatibility between versions are promised."

## Is it access-restricted?

`handle_create_session` in `src/backends/meta-screen-cast.c` (mutter, tag `50.2`) has no sender check, no app-id allowlist, and no polkit call — any process reaches `CreateSession` the same way.

This is confirmed by GNOME developers on the GNOME Discourse thread "Security for Mutter_screencast" (https://discourse.gnome.org/t/security-for-mutter-screencast/35009):

- Jonas Ådahl: "Only users who have access to the D-Bus session bus should be able to use this interface."
- Florian Müllner, noting gnome-shell already restricts some of its D-Bus services to an allowlist of callers, adding mutter "could do something similar" for the services it exports — implying it currently does not, for ScreenCast.
- Adrian Vovk: "xdg-desktop-portal-gnome uses the Mutter ScreenCast API as its backend. That's what the API is there for: to implement the portal," and "There is no way to defend against unsandboxed malicious applications running inside the user session."

So today, on GNOME/Mutter, any unsandboxed process running as the logged-in user (which includes a plain systemd-user daemon like glint) can technically call `RecordVirtual` directly. The "private" label is a compatibility promise GNOME makes to itself, not a technical access barrier enforced against third parties.

## How gnome-remote-desktop actually uses it

Source: `src/grd-session.c` in gnome-remote-desktop at tag `50.2` (https://gitlab.gnome.org/GNOME/gnome-remote-desktop/-/blob/50.2/src/grd-session.c). The function `grd_session_record_virtual()`:

```c
void
grd_session_record_virtual (GrdSession              *session,
                            uint32_t                 stream_id,
                            GrdScreenCastCursorMode  cursor_mode,
                            gboolean                 is_platform)
{
  ...
  g_variant_builder_add (&properties_builder, "{sv}",
                         "cursor-mode", g_variant_new_uint32 (cursor_mode));
  g_variant_builder_add (&properties_builder, "{sv}",
                         "is-platform", g_variant_new_boolean (is_platform));

  grd_dbus_mutter_screen_cast_session_call_record_virtual (priv->screen_cast_session,
                                                           g_variant_builder_end (&properties_builder),
                                                           priv->cancellable,
                                                           on_record_finished,
                                                           async_context);
}
```

This exactly matches the property names documented in the XML. The same file also calls `grd_dbus_mutter_screen_cast_call_create_session()` (ScreenCast session) and `grd_dbus_mutter_remote_desktop_call_create_session()` (companion RemoteDesktop session, used for input injection and to link the screen-cast session via `remote-desktop-session-id`).

## The portal alternative: `org.freedesktop.portal.ScreenCast`, type VIRTUAL

Source: `data/org.freedesktop.portal.ScreenCast.xml` in xdg-desktop-portal, `main` (https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.ScreenCast.xml). The `AvailableSourceTypes` bitmask is documented as:

- `1`: MONITOR — share existing monitors
- `2`: WINDOW — share application windows
- `4`: VIRTUAL — extend with new virtual monitor

A caller passes `types` including bit `4` to `SelectSources`, then calls `Start()`; the result is a PipeWire stream with the same `position`/`size`/`source_type` stream properties used for MONITOR and WINDOW capture.

`xdg-desktop-portal-gnome` added this support in release `42.rc` (source: `NEWS` file, `main` branch, https://github.com/GNOME/xdg-desktop-portal-gnome/blob/main/NEWS — "Support virtual screen cast sources"). Its backend, `src/screencast.c` (https://github.com/GNOME/xdg-desktop-portal-gnome/blob/main/src/screencast.c), routes a `SCREEN_CAST_SOURCE_TYPE_VIRTUAL` selection through `gnome_screen_cast_session_record_selections()`, which per Vovk's statement above is the same underlying call path into Mutter's `RecordVirtual`. So the portal's VIRTUAL type and gnome-remote-desktop's direct `RecordVirtual(is-platform: true)` call are functionally the same feature, just reached through different front doors.

## Implications for a third-party daemon

- glint is a plain (non-Flatpak) daemon, so nothing today technically blocks it from calling `org.gnome.Mutter.ScreenCast.RecordVirtual` directly on the session bus, the same way gnome-remote-desktop does.
- Doing so means depending on an interface GNOME documents as private with "no compatibility between versions promised" — a real risk for a daemon meant to keep working across GNOME point releases and distros without glint's own release cadence tracking mutter's.
- The portal path (`org.freedesktop.portal.ScreenCast`, `types` including `VIRTUAL`) is the GNOME-endorsed way to get the same virtual-monitor capability, works whether or not glint is sandboxed, and is implemented on GNOME using the exact same Mutter primitives underneath — so there is no missing capability, only an added (small) user-consent step.
- Recommendation: build the future "Extend screen" mode against the xdg-desktop-portal ScreenCast portal with `VIRTUAL` requested, and treat direct `org.gnome.Mutter.ScreenCast.RecordVirtual` calls as a fallback only, not the primary path.

## What this research did NOT check

- Other compositors/desktops (KDE Plasma/KWin, wlroots-based compositors, Sway) — this research is GNOME/Mutter-specific; portal VIRTUAL support elsewhere is unverified (the sibling KWin research covers KDE).
- Mutter's separate `org.gnome.Mutter.DisplayConfig.CreateVirtualMonitor` interface and whether it overlaps with or is needed alongside `ScreenCast.RecordVirtual` for the headless-session case; a fetch of `RemoteDesktop.xml` did not return usable content and was not re-verified.
- No live testing against a running GNOME session — all findings are from reading source and docs, not from calling the D-Bus methods.
- Whether `VIRTUAL` portal support exists in non-GNOME portal backends (e.g. xdg-desktop-portal-wlr).
- Whether virtual monitors via either route work under X11 sessions, or are Wayland-only.
- Exact `Version` property / API-version negotiation behavior of `org.gnome.Mutter.ScreenCast` across point releases.
- Whether the portal's VIRTUAL path sets `is-platform` true or false internally, and any resulting UX difference (sharing indicator versus transparent monitor) compared to calling `RecordVirtual` directly.
- Mutter tags newer than `50.2` (mutter itself has `50.3`/`50.4` tags; `50.2` was used to match the most recent tag common to both repos as read on 2026-09-04) were not diffed for changes to this file.

#miracast #research
