/// WiFi Direct (Wi-Fi P2P) Module
///
/// Manages WiFi Direct group state reported by the Kotlin side.
/// WiFi Direct creates an ad-hoc network between Android devices without a router.
/// Once the group is formed, WiFi multicast runs on top of the group's network.
///
/// Architecture:
///   Kotlin (WifiP2pManager) → JNI callbacks → wifi_direct::WifiDirectState
///   StateMachine reads WifiDirectState to decide when to start multicast transport
///
/// WiFi Direct is Android-only. For cross-platform (iPhone, Desktop), devices
/// must be on the same WiFi network and use multicast directly.

use std::net::Ipv4Addr;
use log::info;

/// WiFi Direct group role
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GroupRole {
    /// Not in a group
    None,
    /// This device created the group (acts as the soft-AP)
    Owner,
    /// This device joined an existing group
    Client,
}

/// WiFi Direct connection state (reported by Kotlin BroadcastReceiver)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WifiDirectState {
    /// WiFi Direct not initialized
    Disabled,
    /// WiFi Direct available, not connected
    Available,
    /// Peer discovery running
    Discovering,
    /// Connecting to a peer / forming group
    Connecting,
    /// Group formed, ready for multicast transport
    Connected,
    /// Error state
    Error,
}

/// Discovered WiFi Direct peer (reported by Kotlin)
#[derive(Debug, Clone)]
pub struct WifiDirectPeer {
    pub device_name: String,
    pub device_address: String,
    pub is_group_owner: bool,
}

/// WiFi Direct state tracker
///
/// All state is set by Kotlin via JNI callbacks. The Rust side reads it
/// to decide when the WiFi Direct network is ready for multicast transport.
pub struct WifiDirectManager {
    state: WifiDirectState,
    role: GroupRole,
    /// The group network's IP address (assigned by WifiP2pManager)
    group_owner_address: Option<Ipv4Addr>,
    /// Our IP on the WiFi Direct network
    local_address: Option<Ipv4Addr>,
    /// Peers discovered during WiFi Direct scan
    peers: Vec<WifiDirectPeer>,
    /// The network interface name for the WiFi Direct group (e.g. "p2p-wlan0-0")
    interface_name: Option<String>,
}

impl WifiDirectManager {
    pub fn new() -> Self {
        Self {
            state: WifiDirectState::Disabled,
            role: GroupRole::None,
            group_owner_address: None,
            local_address: None,
            peers: Vec::new(),
            interface_name: None,
        }
    }

    // ── Kotlin → Rust callbacks ──

    /// Called by Kotlin when WiFi Direct state changes
    pub fn on_state_changed(&mut self, enabled: bool) {
        self.state = if enabled {
            WifiDirectState::Available
        } else {
            WifiDirectState::Disabled
        };
        info!("WiFi Direct: state changed to {:?}", self.state);
    }

    /// Called by Kotlin when peer discovery finds devices
    pub fn on_peers_changed(&mut self, peers: Vec<WifiDirectPeer>) {
        info!("WiFi Direct: {} peers discovered", peers.len());
        self.peers = peers;
    }

    /// Called by Kotlin when connection state changes
    pub fn on_connection_changed(&mut self, connected: bool, is_owner: bool, group_owner_ip: Option<Ipv4Addr>, interface: Option<String>) {
        if connected {
            self.state = WifiDirectState::Connected;
            self.role = if is_owner { GroupRole::Owner } else { GroupRole::Client };
            self.group_owner_address = group_owner_ip;
            self.interface_name = interface;
            info!("WiFi Direct: connected as {:?}, GO address: {:?}", self.role, self.group_owner_address);
        } else {
            self.state = WifiDirectState::Available;
            self.role = GroupRole::None;
            self.group_owner_address = None;
            self.local_address = None;
            self.interface_name = None;
            info!("WiFi Direct: disconnected");
        }
    }

    /// Called by Kotlin to report our local IP on the P2P interface
    pub fn set_local_address(&mut self, addr: Ipv4Addr) {
        self.local_address = Some(addr);
        info!("WiFi Direct: local address = {}", addr);
    }

    /// Called when discovery starts
    pub fn on_discovery_started(&mut self) {
        self.state = WifiDirectState::Discovering;
        info!("WiFi Direct: discovery started");
    }

    /// Called when connect initiated
    pub fn on_connecting(&mut self) {
        self.state = WifiDirectState::Connecting;
        info!("WiFi Direct: connecting");
    }

    // ── Rust reads ──

    pub fn get_state(&self) -> WifiDirectState {
        self.state
    }

    pub fn get_role(&self) -> GroupRole {
        self.role
    }

    pub fn is_connected(&self) -> bool {
        self.state == WifiDirectState::Connected
    }

    pub fn group_owner_address(&self) -> Option<Ipv4Addr> {
        self.group_owner_address
    }

    pub fn local_address(&self) -> Option<Ipv4Addr> {
        self.local_address
    }

    pub fn get_peers(&self) -> &[WifiDirectPeer] {
        &self.peers
    }

    pub fn has_peers(&self) -> bool {
        !self.peers.is_empty()
    }

    pub fn interface_name(&self) -> Option<&str> {
        self.interface_name.as_deref()
    }

    /// Reset to initial state
    pub fn reset(&mut self) {
        self.state = WifiDirectState::Disabled;
        self.role = GroupRole::None;
        self.group_owner_address = None;
        self.local_address = None;
        self.peers.clear();
        self.interface_name = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wifi_direct_lifecycle() {
        let mut mgr = WifiDirectManager::new();
        assert_eq!(mgr.get_state(), WifiDirectState::Disabled);

        mgr.on_state_changed(true);
        assert_eq!(mgr.get_state(), WifiDirectState::Available);

        mgr.on_discovery_started();
        assert_eq!(mgr.get_state(), WifiDirectState::Discovering);

        mgr.on_peers_changed(vec![
            WifiDirectPeer {
                device_name: "Galaxy S24".into(),
                device_address: "AA:BB:CC:DD:EE:FF".into(),
                is_group_owner: false,
            },
        ]);
        assert!(mgr.has_peers());
        assert_eq!(mgr.get_peers().len(), 1);

        mgr.on_connecting();
        assert_eq!(mgr.get_state(), WifiDirectState::Connecting);

        mgr.on_connection_changed(
            true,
            true,
            Some(Ipv4Addr::new(192, 168, 49, 1)),
            Some("p2p-wlan0-0".into()),
        );
        assert!(mgr.is_connected());
        assert_eq!(mgr.get_role(), GroupRole::Owner);
        assert_eq!(mgr.group_owner_address(), Some(Ipv4Addr::new(192, 168, 49, 1)));

        mgr.on_connection_changed(false, false, None, None);
        assert!(!mgr.is_connected());
        assert_eq!(mgr.get_role(), GroupRole::None);
    }

    #[test]
    fn test_wifi_direct_reset() {
        let mut mgr = WifiDirectManager::new();
        mgr.on_state_changed(true);
        mgr.on_connection_changed(true, false, Some(Ipv4Addr::new(192, 168, 49, 1)), None);
        assert!(mgr.is_connected());

        mgr.reset();
        assert_eq!(mgr.get_state(), WifiDirectState::Disabled);
        assert_eq!(mgr.get_role(), GroupRole::None);
        assert!(!mgr.has_peers());
    }
}
