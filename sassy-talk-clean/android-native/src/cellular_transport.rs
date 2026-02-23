/// Cellular Transport Module — WebSocket relay via Cloudflare Durable Objects
///
/// Architecture:
///   Kotlin WebSocket client (OkHttp) connects to wss://sassyconsultingllc.com/api/ptt/ws?room=SESSION_ID
///   Binary audio frames flow through a thread-safe ring buffer between Kotlin ↔ Rust:
///
///   TX path: Rust send_audio() → outbound queue → JNI callback → Kotlin WS.send(binary)
///   RX path: Kotlin WS.onMessage(binary) → JNI push → inbound queue → Rust receive_audio()
///
/// The relay is a blind forwarder — it never decrypts. Encryption is handled
/// by TransportManager (AES-256-GCM) before data reaches this module.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use log::{info, warn, error};

/// Max queued packets before dropping oldest
const MAX_QUEUE_SIZE: usize = 64;

/// Max single packet size (encrypted audio frame + overhead)
const MAX_PACKET_SIZE: usize = 1500;

/// Relay server base URL
pub const RELAY_URL: &str = "wss://sassyconsultingllc.com/api/ptt/ws";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellularState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Thread-safe packet queue for WebSocket ↔ audio pipeline
#[derive(Clone)]
pub struct PacketQueue {
    inner: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl PacketQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_QUEUE_SIZE))),
        }
    }

    /// Push a packet. Drops oldest if queue is full.
    pub fn push(&self, data: Vec<u8>) {
        let mut q = self.inner.lock().unwrap();
        if q.len() >= MAX_QUEUE_SIZE {
            q.pop_front(); // Drop oldest to prevent unbounded growth
        }
        q.push_back(data);
    }

    /// Pop a packet (FIFO). Returns None if empty.
    pub fn pop(&self) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().pop_front()
    }

    /// Number of queued packets
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Clear all queued packets
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

/// Cellular transport — bridges audio pipeline with Kotlin WebSocket client
pub struct CellularTransport {
    state: CellularState,
    room_id: String,
    device_name: String,

    /// Inbound queue: Kotlin WS.onMessage → push here → Rust receive_audio() reads
    inbound: PacketQueue,

    /// Outbound queue: Rust send_audio() writes → Kotlin polls and sends via WS
    outbound: PacketQueue,

    /// Stats
    packets_sent: u64,
    packets_received: u64,
}

impl CellularTransport {
    pub fn new(device_name: &str) -> Self {
        Self {
            state: CellularState::Disconnected,
            room_id: String::new(),
            device_name: device_name.to_string(),
            inbound: PacketQueue::new(),
            outbound: PacketQueue::new(),
            packets_sent: 0,
            packets_received: 0,
        }
    }

    /// Set the room ID (derived from QR session_id)
    pub fn set_room_id(&mut self, room_id: String) {
        info!("CellularTransport: room_id set to '{}'", room_id);
        self.room_id = room_id;
    }

    /// Get the full WebSocket URL for Kotlin to connect to
    pub fn get_ws_url(&self) -> String {
        format!("{}?room={}&device={}&client_id={}",
            RELAY_URL,
            self.room_id,
            urlencoded(&self.device_name),
            uuid::Uuid::new_v4()
        )
    }

    /// Get current state
    pub fn get_state(&self) -> CellularState {
        self.state
    }

    /// Get the inbound queue (for JNI to push received packets)
    pub fn inbound_queue(&self) -> &PacketQueue {
        &self.inbound
    }

    /// Get the outbound queue (for JNI to poll outgoing packets)
    pub fn outbound_queue(&self) -> &PacketQueue {
        &self.outbound
    }

    // ── Called by Kotlin via JNI ──

    /// Called when Kotlin WebSocket connects successfully
    pub fn on_connected(&mut self) {
        info!("CellularTransport: WebSocket connected to room '{}'", self.room_id);
        self.state = CellularState::Connected;
        self.inbound.clear();
        self.outbound.clear();
        self.packets_sent = 0;
        self.packets_received = 0;
    }

    /// Called when Kotlin receives a binary message from the relay
    pub fn on_message_received(&mut self, data: Vec<u8>) {
        if data.len() > MAX_PACKET_SIZE {
            warn!("CellularTransport: dropping oversized packet ({} bytes)", data.len());
            return;
        }
        self.packets_received += 1;
        self.inbound.push(data);
    }

    /// Called when Kotlin WebSocket disconnects
    pub fn on_disconnected(&mut self, reason: &str) {
        info!("CellularTransport: disconnected ({})", reason);
        self.state = CellularState::Disconnected;
        self.inbound.clear();
        self.outbound.clear();
    }

    /// Called when Kotlin WebSocket encounters an error
    pub fn on_error(&mut self, error: &str) {
        error!("CellularTransport: WebSocket error: {}", error);
        self.state = CellularState::Error;
    }

    /// Poll outbound queue — called by Kotlin to get next packet to send
    pub fn poll_outbound(&self) -> Option<Vec<u8>> {
        self.outbound.pop()
    }

    // ── Called by TransportManager (audio pipeline) ──

    /// Send encrypted audio data through the WebSocket relay.
    /// Puts data into outbound queue; Kotlin picks it up and sends via WS.
    pub fn send_audio(&mut self, data: &[u8]) -> Result<usize, String> {
        if self.state != CellularState::Connected {
            return Err("Cellular transport not connected".to_string());
        }

        if data.len() > MAX_PACKET_SIZE {
            return Err(format!("Packet too large: {} > {}", data.len(), MAX_PACKET_SIZE));
        }

        self.outbound.push(data.to_vec());
        self.packets_sent += 1;
        Ok(data.len())
    }

    /// Receive encrypted audio data from the WebSocket relay.
    /// Reads from inbound queue (filled by Kotlin WS.onMessage).
    pub fn receive_audio(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        if self.state != CellularState::Connected {
            return Ok(0);
        }

        match self.inbound.pop() {
            Some(packet) => {
                let copy_len = packet.len().min(buffer.len());
                buffer[..copy_len].copy_from_slice(&packet[..copy_len]);
                Ok(copy_len)
            }
            None => Ok(0), // No data available — non-blocking
        }
    }

    /// Activate the transport (called after Kotlin connects)
    pub fn activate(&mut self) {
        self.state = CellularState::Connected;
    }

    /// Shutdown the transport
    pub fn shutdown(&mut self) {
        info!("CellularTransport: shutting down");
        self.state = CellularState::Disconnected;
        self.inbound.clear();
        self.outbound.clear();
    }

    /// Get stats as JSON string
    pub fn get_stats(&self) -> String {
        format!(
            r#"{{"state":"{}","room":"{}","sent":{},"received":{},"inbound_queue":{},"outbound_queue":{}}}"#,
            match self.state {
                CellularState::Disconnected => "disconnected",
                CellularState::Connecting => "connecting",
                CellularState::Connected => "connected",
                CellularState::Error => "error",
            },
            self.room_id,
            self.packets_sent,
            self.packets_received,
            self.inbound.len(),
            self.outbound.len()
        )
    }
}

/// Simple percent-encoding for URL query params
fn urlencoded(s: &str) -> String {
    s.chars().map(|c| {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            _ => format!("%{:02X}", c as u8),
        }
    }).collect()
}
