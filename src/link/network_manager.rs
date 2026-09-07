//! The `P2pLink` implementation that drives NetworkManager over D-Bus.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::link::{GroupId, LinkError, LinkHandle, P2pLink, Peer};
use crate::receiver::MacAddr;

/// NetworkManager's numeric device type for a Wi-Fi Direct device. The P2P
/// device's object path is not a stable invariant across NetworkManager
/// restarts, so it is resolved by this type every time a link is built and
/// never hard-coded.
const DEVICE_TYPE_WIFI_P2P: u32 = 30;

/// How long a `scan` collects peers. NetworkManager's own find times out
/// at 30 seconds by default, so the window sits well under it and the find
/// then lapses on its own.
const SCAN_WINDOW: Duration = Duration::from_secs(10);

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_all_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn add_and_activate_connection2(
        &self,
        connection: &ConnectionSettings,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedValue>,
    )>;

    fn deactivate_connection(&self, active_connection: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager",
    assume_defaults = false
)]
trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

/// `StopFind` is deliberately absent. GND's own FIXME records that calling
/// it around a state change makes the connection fail, so glint lets a
/// find lapse on NetworkManager's timeout instead; leaving the method off
/// the proxy makes calling it impossible rather than merely discouraged.
#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.WifiP2P",
    default_service = "org.freedesktop.NetworkManager",
    assume_defaults = false
)]
trait WifiP2PDevice {
    fn start_find(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property)]
    fn peers(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.WifiP2PPeer",
    default_service = "org.freedesktop.NetworkManager",
    assume_defaults = false
)]
trait WifiP2PPeer {
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;

    /// Named explicitly because the macro would derive `WfdIes` from the
    /// method name, and NetworkManager spells the property `WfdIEs`.
    #[zbus(property, name = "WfdIEs")]
    fn wfd_ies(&self) -> zbus::Result<Vec<u8>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager",
    assume_defaults = false
)]
trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;

    fn delete(&self) -> zbus::Result<()>;
}

/// One WFD Device Information subelement declaring glint a Wi-Fi Display
/// source with its RTSP server on port 7236. Copied verbatim from
/// gnome-network-displays (nd-wfd-p2p-sink.c), field-proven against real
/// sinks since 2019: subelement id 0, length 6, device-info bitfield 0x0090
/// (source, available-for-session, and bit 7 = TDLS-preferred, which is
/// GND's own value and is unexplained upstream — kept because it is what
/// sinks have actually accepted, so do not "correct" it without testing
/// against a real sink), control port 0x1c44 = 7236, max throughput
/// 0x00c8 = 200.
const WFD_SOURCE_IES: [u8; 9] = [0x00, 0x00, 0x06, 0x00, 0x90, 0x1c, 0x44, 0x00, 0xc8];

/// A peer's three properties as NetworkManager reports them, before glint
/// has decided whether the peer is a Wi-Fi Display sink at all.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawPeer {
    name: String,
    hw_address: String,
    wfd_ies: Vec<u8>,
}

/// Keeping only peers that advertise Wi-Fi Display information elements is
/// GND's rule for telling a sink from any other Wi-Fi Direct neighbour. A
/// peer whose address will not parse is skipped rather than propagated:
/// one malformed neighbour must not fail a whole scan.
fn wfd_peers(seen: &[RawPeer]) -> Vec<Peer> {
    seen.iter()
        .filter(|raw| !raw.wfd_ies.is_empty())
        .filter_map(|raw| {
            MacAddr::from_str(&raw.hw_address).ok().map(|mac| Peer {
                mac,
                name: raw.name.clone(),
            })
        })
        .collect()
}

/// Compares through `MacAddr` rather than by string so NetworkManager's
/// uppercase `HwAddress` matches glint's canonical lowercase form.
fn resolve_peer_path<P: Clone>(peers: &[(String, P)], mac: MacAddr) -> Option<P> {
    peers
        .iter()
        .find(|(hw_address, _)| MacAddr::from_str(hw_address) == Ok(mac))
        .map(|(_, path)| path.clone())
}

/// The one prefix both readers use: connect names a connection with it and
/// startup cleanup filters on it, so the two can never drift apart.
const GLINT_CONNECTION_PREFIX: &str = "glint p2p ";

/// NetworkManager's `a{sa{sv}}` connection profile. Built by a pure
/// function so its structure is testable without a bus, which matters
/// because a wrong variant type here fails only against a live
/// NetworkManager.
type ConnectionSettings = HashMap<&'static str, HashMap<&'static str, Value<'static>>>;

/// Naming the connection after its peer is deliberate belt and braces
/// against the volatile, bus-bound activation that connect asks for:
/// NetworkManager drops the connection when glint's D-Bus connection dies,
/// and the `glint p2p ` id lets startup remove anything that outlived a
/// hard crash. Neither mechanism is trusted on its own.
fn connection_settings(mac: MacAddr, ies: &[u8]) -> ConnectionSettings {
    HashMap::from([
        (
            "connection",
            HashMap::from([
                ("id", Value::from(format!("{GLINT_CONNECTION_PREFIX}{mac}"))),
                ("type", Value::from("wifi-p2p")),
            ]),
        ),
        (
            "wifi-p2p",
            HashMap::from([("wfd-ies", Value::from(ies.to_vec()))]),
        ),
        (
            "ipv4",
            HashMap::from([
                ("method", Value::from("auto")),
                ("never-default", Value::from(true)),
            ]),
        ),
        (
            "ipv6",
            HashMap::from([
                ("method", Value::from("auto")),
                ("never-default", Value::from(true)),
                ("may-fail", Value::from(true)),
            ]),
        ),
    ])
}

/// A prefix test, not a substring test: a connection that merely mentions
/// the phrase mid-id belongs to someone else and must not be deleted.
fn stale_connection_paths<P: Clone>(connections: &[(String, P)]) -> Vec<P> {
    connections
        .iter()
        .filter(|(id, _)| id.starts_with(GLINT_CONNECTION_PREFIX))
        .map(|(_, path)| path.clone())
        .collect()
}

pub struct NetworkManagerLink {
    connection: zbus::Connection,
    device_path: OwnedObjectPath,
    /// `LinkHandle` is opaque to callers, so the active-connection path it
    /// stands for has to be recoverable here for `disconnect` to undo
    /// exactly the activation `connect` made.
    activations: Mutex<HashMap<LinkHandle, OwnedObjectPath>>,
    next_handle: Mutex<u64>,
}

/// zbus errors are stringified rather than wrapped because `LinkError`
/// derives `Eq`, and a `#[from] zbus::Error` variant would forbid that.
fn backend(error: impl std::fmt::Display) -> LinkError {
    LinkError::Backend(error.to_string())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl NetworkManagerLink {
    pub async fn connect() -> Result<Self, LinkError> {
        let connection = zbus::Connection::system().await.map_err(backend)?;
        let manager = ManagerProxy::new(&connection).await.map_err(backend)?;
        let mut wifi_p2p = None;
        for path in manager.get_all_devices().await.map_err(backend)? {
            let device = DeviceProxy::builder(&connection)
                .path(&path)
                .map_err(backend)?
                .build()
                .await
                .map_err(backend)?;
            if device.device_type().await.map_err(backend)? == DEVICE_TYPE_WIFI_P2P {
                wifi_p2p = Some(path);
                break;
            }
        }
        let device_path =
            wifi_p2p.ok_or_else(|| LinkError::Backend("no Wi-Fi P2P device".to_string()))?;
        Ok(NetworkManagerLink {
            connection,
            device_path,
            activations: Mutex::new(HashMap::new()),
            next_handle: Mutex::new(0),
        })
    }

    async fn device(&self) -> Result<WifiP2PDeviceProxy<'_>, LinkError> {
        WifiP2PDeviceProxy::builder(&self.connection)
            .path(&self.device_path)
            .map_err(backend)?
            .build()
            .await
            .map_err(backend)
    }

    /// Takes the path by value because a proxy borrowing both the
    /// connection and a caller-owned path cannot outlive either.
    async fn peer(&self, path: OwnedObjectPath) -> Result<WifiP2PPeerProxy<'_>, LinkError> {
        WifiP2PPeerProxy::builder(&self.connection)
            .path(path)
            .map_err(backend)?
            .build()
            .await
            .map_err(backend)
    }

    async fn profile(
        &self,
        path: OwnedObjectPath,
    ) -> Result<SettingsConnectionProxy<'_>, LinkError> {
        SettingsConnectionProxy::builder(&self.connection)
            .path(path)
            .map_err(backend)?
            .build()
            .await
            .map_err(backend)
    }

    fn mint_handle(&self) -> LinkHandle {
        let mut next = lock(&self.next_handle);
        *next += 1;
        LinkHandle::new(*next)
    }
}

impl P2pLink for NetworkManagerLink {
    async fn scan(&self) -> Result<Vec<Peer>, LinkError> {
        let device = self.device().await?;
        device.start_find(HashMap::new()).await.map_err(backend)?;
        // Sleep the whole window and read `Peers` once at the end, rather
        // than polling and returning early on the first hit: NetworkManager
        // accumulates peers for as long as the find runs, so one late read
        // sees everything a poll loop would have, and every sink gets the
        // full window to answer. An early-return loop is the tempting
        // "optimization" here and it can return before the TV has replied.
        tokio::time::sleep(SCAN_WINDOW).await;
        let mut seen = Vec::new();
        for path in device.peers().await.map_err(backend)? {
            let peer = self.peer(path).await?;
            seen.push(RawPeer {
                name: peer.name().await.map_err(backend)?,
                hw_address: peer.hw_address().await.map_err(backend)?,
                wfd_ies: peer.wfd_ies().await.map_err(backend)?,
            });
        }
        Ok(wfd_peers(&seen))
    }

    /// `Peers` is re-read rather than carried over from `scan`, because
    /// peer objects expire once a find lapses.
    async fn connect(&self, peer: &Peer) -> Result<LinkHandle, LinkError> {
        let device = self.device().await?;
        let mut visible = Vec::new();
        for path in device.peers().await.map_err(backend)? {
            let hw_address = self
                .peer(path.clone())
                .await?
                .hw_address()
                .await
                .map_err(backend)?;
            visible.push((hw_address, path));
        }
        let peer_path =
            resolve_peer_path(&visible, peer.mac).ok_or(LinkError::PeerUnreachable(peer.mac))?;

        // The peer travels in `specific_object` rather than in the
        // profile's `wifi-p2p.peer` field: that is what GND's field-proven
        // code does, contradicting the man page's "only way" wording.
        // `bind-activation` plus `persist = volatile` make NetworkManager
        // drop the connection when glint's own bus connection dies.
        let (_profile, activation, _result) = ManagerProxy::new(&self.connection)
            .await
            .map_err(backend)?
            .add_and_activate_connection2(
                &connection_settings(peer.mac, &WFD_SOURCE_IES),
                &self.device_path.as_ref(),
                &peer_path.as_ref(),
                HashMap::from([
                    ("bind-activation", Value::from("dbus-client")),
                    ("persist", Value::from("volatile")),
                ]),
            )
            .await
            .map_err(backend)?;

        let handle = self.mint_handle();
        lock(&self.activations).insert(handle, activation);
        Ok(handle)
    }

    /// Deactivating the activation glint made, rather than disconnecting
    /// the device as GND does, is the precise inverse of `connect` and
    /// leaves any unrelated activation on the same device alone.
    async fn disconnect(&self, handle: LinkHandle) -> Result<(), LinkError> {
        let activation = lock(&self.activations)
            .remove(&handle)
            .ok_or_else(|| LinkError::Backend("unknown link handle".to_string()))?;
        ManagerProxy::new(&self.connection)
            .await
            .map_err(backend)?
            .deactivate_connection(&activation.as_ref())
            .await
            .map_err(backend)
    }

    /// Filters by id prefix rather than by liveness: whether an active
    /// volatile connection is even listed here is unmeasured, so the
    /// prefix is the only signal trusted.
    async fn stale_groups(&self) -> Result<Vec<GroupId>, LinkError> {
        let settings = SettingsProxy::new(&self.connection)
            .await
            .map_err(backend)?;
        let mut named = Vec::new();
        for path in settings.list_connections().await.map_err(backend)? {
            let sections = self
                .profile(path.clone())
                .await?
                .get_settings()
                .await
                .map_err(backend)?;
            let id = sections
                .get("connection")
                .and_then(|section| section.get("id"))
                .and_then(|id| String::try_from(id.clone()).ok());
            if let Some(id) = id {
                named.push((id, path));
            }
        }
        Ok(stale_connection_paths(&named)
            .into_iter()
            .map(|path| GroupId::new(path.as_str()))
            .collect())
    }

    async fn remove_group(&self, id: GroupId) -> Result<(), LinkError> {
        let path = OwnedObjectPath::try_from(id.as_str()).map_err(backend)?;
        match self.profile(path).await?.delete().await {
            Ok(()) => Ok(()),
            // A profile that has already gone is the desired end state, so
            // a repeated removal is not an error. A malformed id is still
            // reported: it names nothing that ever existed, and swallowing
            // it would hide a caller's bug.
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.UnknownObject"
                        | "org.freedesktop.DBus.Error.UnknownMethod"
                        | "org.freedesktop.DBus.Error.UnknownInterface"
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(backend(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, hw_address: &str, wfd_ies: &[u8]) -> RawPeer {
        let advertised = wfd_ies.to_vec();
        RawPeer {
            name: name.to_string(),
            hw_address: hw_address.to_string(),
            wfd_ies: advertised,
        }
    }

    fn peer(mac: &str, name: &str) -> Peer {
        Peer {
            mac: mac.parse().unwrap(),
            name: name.to_string(),
        }
    }

    #[test]
    fn the_filter_keeps_only_peers_advertising_wfd_ies() {
        // arrange
        let seen = vec![
            raw("TV", "AA:BB:CC:DD:EE:FF", &WFD_SOURCE_IES),
            raw("Someone's phone", "00:11:22:33:44:55", &[]),
            raw("Beamer", "01:02:03:04:05:06", &[0x00, 0x00, 0x06]),
        ];
        // act
        let kept = wfd_peers(&seen);
        // assert
        assert_eq!(
            kept,
            vec![
                peer("aa:bb:cc:dd:ee:ff", "TV"),
                peer("01:02:03:04:05:06", "Beamer"),
            ]
        );
    }

    #[test]
    fn a_peer_whose_hardware_address_will_not_parse_is_dropped_not_fatal() {
        // arrange
        let seen = vec![
            raw("Rubbish", "not-a-mac", &WFD_SOURCE_IES),
            raw("TV", "AA:BB:CC:DD:EE:FF", &WFD_SOURCE_IES),
        ];
        // act
        let kept = wfd_peers(&seen);
        // assert
        assert_eq!(kept, vec![peer("aa:bb:cc:dd:ee:ff", "TV")]);
    }

    #[test]
    fn resolving_a_present_mac_returns_its_peer_path() {
        // arrange
        let peers = vec![
            ("00:11:22:33:44:55".to_string(), "/peers/1"),
            ("aa:bb:cc:dd:ee:ff".to_string(), "/peers/2"),
        ];
        // act
        let found = resolve_peer_path(&peers, "aa:bb:cc:dd:ee:ff".parse().unwrap());
        // assert
        assert_eq!(found, Some("/peers/2"));
    }

    #[test]
    fn resolving_an_absent_mac_returns_none() {
        // arrange
        let peers = vec![("00:11:22:33:44:55".to_string(), "/peers/1")];
        // act
        let found = resolve_peer_path(&peers, "aa:bb:cc:dd:ee:ff".parse().unwrap());
        // assert
        assert_eq!(found, None);
    }

    #[test]
    fn an_uppercase_hardware_address_resolves_against_a_canonical_mac() {
        // arrange
        let peers = vec![("AA:BB:CC:DD:EE:FF".to_string(), "/peers/2")];
        // act
        let found = resolve_peer_path(&peers, "aa:bb:cc:dd:ee:ff".parse().unwrap());
        // assert
        assert_eq!(found, Some("/peers/2"));
    }

    #[test]
    fn connection_settings_names_the_connection_after_its_peer() {
        // arrange
        let cases = [
            ("aa:bb:cc:dd:ee:ff", "glint p2p aa:bb:cc:dd:ee:ff"),
            ("00:11:22:33:44:55", "glint p2p 00:11:22:33:44:55"),
        ];
        for (mac, expected_id) in cases {
            // act
            let settings = connection_settings(mac.parse().unwrap(), &WFD_SOURCE_IES);
            // assert
            assert_eq!(
                settings["connection"]["id"],
                Value::from(expected_id),
                "id for {mac}"
            );
        }
    }

    #[test]
    fn connection_settings_declares_the_wifi_p2p_type() {
        // arrange
        let mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        // act
        let settings = connection_settings(mac, &WFD_SOURCE_IES);
        // assert
        assert_eq!(settings["connection"]["type"], Value::from("wifi-p2p"));
    }

    #[test]
    fn connection_settings_carries_non_empty_wfd_ies() {
        // Deliberately not compared against WFD_SOURCE_IES: this row guards
        // only that the load-bearing field is wired in, so that flipping a
        // byte of the constant reddens the byte-equality test alone.
        // arrange
        let mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        // act
        let settings = connection_settings(mac, &WFD_SOURCE_IES);
        // assert
        let ies = settings["wifi-p2p"]
            .get("wfd-ies")
            .expect("wfd-ies must be present");
        assert!(
            matches!(ies, Value::Array(bytes) if !bytes.is_empty()),
            "wfd-ies must be a non-empty byte array, got {ies:?}"
        );
    }

    #[test]
    fn connection_settings_keeps_the_cast_off_the_default_route() {
        // arrange
        let mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        // act
        let settings = connection_settings(mac, &WFD_SOURCE_IES);
        // assert
        assert_eq!(settings["ipv4"]["method"], Value::from("auto"));
        assert_eq!(settings["ipv4"]["never-default"], Value::from(true));
        assert_eq!(settings["ipv6"]["method"], Value::from("auto"));
        assert_eq!(settings["ipv6"]["never-default"], Value::from(true));
        assert_eq!(settings["ipv6"]["may-fail"], Value::from(true));
    }

    #[test]
    fn the_stale_filter_keeps_only_ids_carrying_the_glint_prefix() {
        // arrange
        let connections = vec![
            ("NSI-5G".to_string(), "/settings/1"),
            ("glint p2p aa:bb:cc:dd:ee:ff".to_string(), "/settings/2"),
            ("lo".to_string(), "/settings/3"),
            ("glint p2p 00:11:22:33:44:55".to_string(), "/settings/4"),
        ];
        // act
        let stale = stale_connection_paths(&connections);
        // assert
        assert_eq!(stale, vec!["/settings/2", "/settings/4"]);
    }

    #[test]
    fn an_id_that_merely_contains_the_prefix_is_not_ours() {
        // arrange
        let connections = vec![("not the glint p2p one".to_string(), "/settings/9")];
        // act
        let stale = stale_connection_paths(&connections);
        // assert
        assert!(stale.is_empty());
    }

    #[test]
    fn wfd_source_ies_are_gnds_exact_nine_bytes() {
        // The sole oracle for the bytes: every other test asserts only that
        // the field is wired in, so flipping a byte here reddens this test
        // and nothing else.
        // arrange, act, assert
        assert_eq!(
            WFD_SOURCE_IES,
            [0x00, 0x00, 0x06, 0x00, 0x90, 0x1c, 0x44, 0x00, 0xc8]
        );
    }
}
