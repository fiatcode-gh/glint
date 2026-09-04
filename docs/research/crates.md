# R5: Dependency versions to pin

**Verdict:** All six crates (or crate pairs) are actively maintained as of September 2026. Pin `gstreamer` 0.25.3, `gstreamer-rtsp-server` 0.25.2, `zbus` 5.19.0, `ashpd` 0.13.13, and — for Secret Service — `oo7` 0.6.0 (recommended over `secret-service`). No crate here is abandoned or archived.

Checked 2026-09-04. Versions were verified twice: by a research agent and independently against the crates.io API by the architect session.

## Versions and maintenance evidence

| Crate | Pin | Released | Repository | Last activity | Open issues | Backed by |
|---|---|---|---|---|---|---|
| gstreamer | 0.25.3 | 2026-06-29 | gitlab.freedesktop.org/gstreamer/gstreamer-rs | 2026-09-01 (GitHub mirror push) | not readable (GitLab blocked scraping; see caveat) | Sebastian Dröge, under the freedesktop.org / GStreamer project umbrella |
| gstreamer-rtsp-server | 0.25.2 | 2026-05-11 | same repo (gstreamer-rs monorepo) | same as above | same as above | same |
| zbus | 5.19.0 | 2026-08-09 | github.com/z-galaxy/zbus | 2026-09-03 (last push) | 96 | z-galaxy org (the repository moved here from github.com/dbus2/zbus; the crates.io metadata for 5.19.0 already points at the new URL) |
| ashpd | 0.13.13 | 2026-07-17 | github.com/bilelmoussaoui/ashpd | 2026-09-02 (last push) | 18 | Bilel Moussaoui, individual maintainer (a core GNOME/freedesktop contributor) |
| oo7 | 0.6.0 | 2026-02-21 | github.com/linux-credentials/oo7 | 2026-09-02 (last push) | 8 | linux-credentials org |
| secret-service | 5.2.0 | 2026-08-29 | github.com/open-source-cooperative/secret-service-rs | 2026-08-30 (last push) | 5 | open-source-cooperative org (the old github.com/hwchen/secret-service-rs URL now redirects here; crates.io metadata still shows the old URL) |

Source for every version and date above: the crates.io API (`crates.io/api/v1/crates/<name>` and `.../versions`), fetched today. Repository activity and issue counts came from the GitHub API (`api.github.com/repos/<owner>/<repo>`) and, for gstreamer-rs, the GitHub mirror at `github.com/sdroege/gstreamer-rs` (its description confirms it is a read-only mirror pointing at the GitLab repository).

Caveat: `gitlab.freedesktop.org` sits behind an anti-bot check (Anubis) that blocked automated fetches, including the GitLab API. Its GitHub mirror substitutes as a freshness signal — a push on 2026-09-01 is consistent with active upstream development — but issue-tracker counts for the GitLab-hosted repo were not obtained directly.

`gstreamer-rtsp-server` does ship a `subclass` module (confirmed on docs.rs for 0.25.2) — a sibling research task (R6, RTSP approach) needs this fact; only its existence is recorded here.

Update, same day: the RTSP research (`rtsp-approach.md`) concluded glint will not use gst-rtsp-server at all — its subclass module is missing hooks Wi-Fi Display needs. The `gstreamer-rtsp-server` row above stays for the record, but the crate leaves the dependency set; `rtsp-types` 0.1.3 (and tokio) join it instead. This does not change the MSRV floor (the `gstreamer` crate already requires Rust 1.92).

**Version coupling:** gstreamer-rs is one Cargo workspace where every crate's `version` and `rust-version` are set via `version.workspace = true` — they are released together and meant to stay on the same version line. In practice `gstreamer-rtsp-server` is one patch behind (0.25.2 vs core's 0.25.3, both on the 0.25 line); pin both crates to `0.25.2` for an exact match, or accept the one-patch gap if the core crate should have 0.25.3's fixes.

**GStreamer C library requirement:** the bindings need GStreamer and gst-plugins-base >= 1.14 at build time (stated in the gstreamer-rs README). Feature flags `v1_16` through `v1_30` are cumulative and gate access to APIs added in each release. The development host runs GStreamer 1.28.6, so the project should enable the `v1_28` feature to use APIs added up through 1.28.

**Minimum supported Rust version (MSRV):** the highest MSRV in this set is Rust 1.92, required by `gstreamer`, `gstreamer-rtsp-server`, and `oo7` (per each crate's `rust_version` field on crates.io). `zbus`, `ashpd`, and `secret-service` need only 1.87. Since Cargo enforces the highest MSRV among dependencies, the effective floor for glint is **Rust 1.92**.

One forward-looking note: zbus's `main` branch already carries `version = "6.0.0"` in its workspace `Cargo.toml`, ahead of the published 5.19.0 — a major rewrite is in progress upstream but not yet released. Pinning 5.x is safe today; revisit before a 6.0 release lands.

## oo7 vs secret-service: recommendation

**Recommend `oo7` 0.6.0.**

Both crates are actively maintained, both are async, and both support the create/search/delete-by-attribute pattern glint needs for one pairing secret. The deciding factors:

- **Backend fit.** `oo7` auto-selects between the D-Bus Secret Service protocol and a file-based keyring compatible with libsecret, with portal integration for sandboxed apps (the `org.freedesktop.impl.portal.Secret` interface). `secret-service` only speaks the D-Bus Secret Service protocol — no fallback if no Secret Service daemon is running. Since glint already depends on xdg-desktop-portal for screen capture, `oo7`'s portal-aware backend keeps the whole daemon consistent with one integration model instead of two.
- **Maintenance backing.** `oo7` is maintained under the `linux-credentials` GitHub org (multiple maintainers); `secret-service` recently moved from a single maintainer (`hwchen`) to the `open-source-cooperative` org, which is a positive sign but younger as an org home.
- **API shape.** `oo7`'s `Keyring` type offers `create_item`, `search_items`, and `delete`, all keyed by an attributes map — a direct match for storing one pairing secret keyed by, for example, a device identifier.

Both crates are viable; `secret-service` is the safer pick if the project ever needs a synchronous API (it ships a `blocking` module) or wants to avoid the file-backend code path entirely.

## What this research did NOT check

- Whether these crates actually compile and link against the host's GStreamer 1.28.6 installation — no build was attempted.
- Security audit or CVE history for any crate.
- License compatibility beyond noting `gstreamer`'s dual MIT/Apache-2.0 license; the others were not individually checked.
- The exact GitLab issue count and CI health for gstreamer-rs (blocked by anti-bot protection).
- Whether `gstreamer-rtsp-server`'s `subclass` module covers glint's specific media-factory needs — left to the sibling research task that depends on subclassing.
- The size or health of each crate's downstream dependent ecosystem beyond raw download counts.
- zbus's in-progress 6.0 rewrite in any depth — only its existence on the `main` branch was confirmed, not its scope or timeline.

#miracast #research #rust
