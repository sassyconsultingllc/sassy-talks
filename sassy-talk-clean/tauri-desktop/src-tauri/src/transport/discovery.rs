/// Discovery Service - Peer Discovery and Management

use super::{TransportError, BEACON_INTERVAL_SECS};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{self, Duration};
use tracing::info;

/// Discovery service for managing peer discovery
pub struct DiscoveryService {
    peers: Arc<RwLock<HashMap<u32, super::PeerInfo>>>,
    device_id: u32,
}

impl DiscoveryService {
    /// Create new discovery service
    pub fn new(device_id: u32) -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            device_id,
        }
    }
    
    /// Start discovery service
    pub async fn start(&self) {
        info!("Discovery service started");
        
        let peers = Arc::clone(&self.peers);
        
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(BEACON_INTERVAL_SECS));
            
            loop {
                interval.tick().await;
                
                // Clean up stale peers
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                
                peers.write().unwrap().retain(|_, peer| {
                    peer.is_active()
                });
            }
        });
    }
    
    /// Add or update peer
    pub fn update_peer(&self, peer: super::PeerInfo) {
        self.peers.write().unwrap().insert(peer.device_id, peer);
    }
    
    /// Get all active peers
    pub fn get_peers(&self) -> Vec<super::PeerInfo> {
        self.peers.read().unwrap()
            .values()
            .filter(|p| p.is_active())
            .cloned()
            .collect()
    }
    
    /// Get peer by device ID
    pub fn get_peer(&self, device_id: u32) -> Option<super::PeerInfo> {
        self.peers.read().unwrap().get(&device_id).cloned()
    }
}
