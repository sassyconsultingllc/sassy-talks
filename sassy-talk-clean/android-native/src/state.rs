/// State Machine - Central coordinator for all subsystems
///
/// Manages audio engine, transport, crypto, users, audio cache, and the
/// TX/RX audio pipeline threads. Provides the API surface consumed by
/// both the JNI exports (Kotlin app) and the legacy egui UI.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use log::{info, warn};

use crate::audio::AudioEngine;
use crate::audio_pipeline;
use crate::transport::{TransportManager, ActiveTransport};
use crate::audio_cache::AudioCache;
use crate::users::UserRegistry;
use crate::crypto::CryptoSession;
use crate::wifi_direct::{WifiDirectState, WifiDirectPeer, GroupRole};
use crate::cellular_transport::CellularState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Initializing,
    Ready,
    Connecting,
    Connected,
    Transmitting,
    Receiving,
    Disconnecting,
    Error,
}

pub struct StateMachine {
    state: Arc<Mutex<AppState>>,
    transport: Arc<Mutex<TransportManager>>,
    audio: Arc<Mutex<AudioEngine>>,
    audio_cache: Arc<Mutex<AudioCache>>,
    user_registry: Arc<Mutex<UserRegistry>>,
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    tx_running: Arc<AtomicBool>,
    rx_running: Arc<AtomicBool>,
    device_name: String,
    local_sender_id: String,
}

impl StateMachine {
    pub fn new(ptt: Arc<AtomicBool>, channel: Arc<AtomicU8>) -> Self {
        let device_name = "SassyTalkie-Android".to_string();
        let local_sender_id = crate::users::UserRegistry::derive_user_id(device_name.as_bytes());

        Self {
            state: Arc::new(Mutex::new(AppState::Initializing)),
            transport: Arc::new(Mutex::new(TransportManager::new(&device_name).unwrap())),
            audio: Arc::new(Mutex::new(AudioEngine::new().unwrap())),
            audio_cache: Arc::new(Mutex::new(AudioCache::new())),
            user_registry: Arc::new(Mutex::new(UserRegistry::new())),
            ptt_pressed: ptt,
            current_channel: channel,
            tx_running: Arc::new(AtomicBool::new(false)),
            rx_running: Arc::new(AtomicBool::new(false)),
            device_name,
            local_sender_id,
        }
    }

    pub fn initialize(&self) -> Result<(), String> {
        info!("StateMachine: initializing");
        let audio = self.audio.lock().unwrap();
        audio.init_recorder()?;
        audio.init_player()?;
        *self.state.lock().unwrap() = AppState::Ready;
        info!("StateMachine: ready");
        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), String> {
        info!("StateMachine: shutting down");
        self.stop_audio_threads();
        self.disconnect()?;
        let audio = self.audio.lock().unwrap();
        let _ = audio.release();
        Ok(())
    }

    // ── Audio Pipeline Threads ──

    /// Start TX and RX threads. Called when transport is ready.
    fn start_audio_pipeline(&self) {
        if self.tx_running.load(Ordering::SeqCst) {
            return; // Already running
        }

        self.tx_running.store(true, Ordering::SeqCst);
        self.rx_running.store(true, Ordering::SeqCst);

        audio_pipeline::spawn_tx_thread(
            Arc::clone(&self.tx_running),
            Arc::clone(&self.ptt_pressed),
            Arc::clone(&self.current_channel),
            Arc::clone(&self.audio),
            Arc::clone(&self.transport),
            self.local_sender_id.clone(),
            self.device_name.clone(),
        );

        audio_pipeline::spawn_rx_thread(
            Arc::clone(&self.rx_running),
            Arc::clone(&self.current_channel),
            Arc::clone(&self.audio),
            Arc::clone(&self.transport),
            Arc::clone(&self.audio_cache),
            Arc::clone(&self.user_registry),
        );

        info!("StateMachine: audio pipeline started");
    }

    /// Stop TX and RX threads.
    fn stop_audio_threads(&self) {
        self.tx_running.store(false, Ordering::SeqCst);
        self.rx_running.store(false, Ordering::SeqCst);
        // Threads will exit their loops on next iteration check
        info!("StateMachine: audio pipeline stop signaled");
    }

    // ── WiFi Direct Connection (Android-to-Android, no router) ──

    /// Called by Kotlin JNI when WiFi Direct group is formed.
    /// Starts multicast transport on the P2P network and begins audio pipeline.
    pub fn on_wifi_direct_connected(&self) -> Result<(), String> {
        info!("StateMachine: WiFi Direct group formed");

        {
            let mut transport = self.transport.lock().unwrap();
            transport.on_wifi_direct_connected()?;
        }

        *self.state.lock().unwrap() = AppState::Connected;
        self.start_audio_pipeline();
        Ok(())
    }

    /// Called by Kotlin JNI when WiFi Direct group is dissolved.
    pub fn on_wifi_direct_disconnected(&self) {
        info!("StateMachine: WiFi Direct group dissolved");
        self.stop_audio_threads();

        {
            let mut transport = self.transport.lock().unwrap();
            transport.on_wifi_direct_disconnected();
        }

        let current = *self.state.lock().unwrap();
        if current == AppState::Connected || current == AppState::Transmitting || current == AppState::Receiving {
            *self.state.lock().unwrap() = AppState::Ready;
        }
    }

    // ── WiFi Multicast Connection (cross-platform, shared WiFi) ──

    /// Start WiFi multicast transport directly (for cross-platform use).
    /// Call this when devices are on the same WiFi network (no WiFi Direct needed).
    pub fn connect_wifi_multicast(&self) -> Result<(), String> {
        info!("StateMachine: connecting via WiFi multicast (cross-platform)");
        *self.state.lock().unwrap() = AppState::Connecting;

        {
            let mut transport = self.transport.lock().unwrap();
            transport.connect_wifi_multicast().map_err(|e| {
                *self.state.lock().unwrap() = AppState::Error;
                e
            })?;
        }

        *self.state.lock().unwrap() = AppState::Connected;
        self.start_audio_pipeline();
        info!("StateMachine: WiFi multicast connected, audio pipeline started");
        Ok(())
    }

    // ── Cellular Connection (WebSocket relay, works anywhere with internet) ──

    /// Set the cellular relay room ID (from QR session_id)
    pub fn set_cellular_room(&self, room_id: String) {
        let mut transport = self.transport.lock().unwrap();
        transport.set_cellular_room(room_id);
    }

    /// Get the WebSocket URL for Kotlin to connect to
    pub fn get_cellular_ws_url(&self) -> String {
        self.transport.lock().unwrap().get_cellular_ws_url()
    }

    /// Called by Kotlin JNI when the cellular WebSocket connects
    pub fn on_cellular_connected(&self) -> Result<(), String> {
        info!("StateMachine: cellular WebSocket connected");

        {
            let mut transport = self.transport.lock().unwrap();
            transport.on_cellular_connected()?;
        }

        *self.state.lock().unwrap() = AppState::Connected;
        self.start_audio_pipeline();
        info!("StateMachine: cellular connected, audio pipeline started");
        Ok(())
    }

    /// Called by Kotlin JNI when the cellular WebSocket disconnects
    pub fn on_cellular_disconnected(&self, reason: &str) {
        info!("StateMachine: cellular disconnected: {}", reason);
        self.stop_audio_threads();

        {
            let mut transport = self.transport.lock().unwrap();
            transport.on_cellular_disconnected(reason);
        }

        let current = *self.state.lock().unwrap();
        if current == AppState::Connected || current == AppState::Transmitting || current == AppState::Receiving {
            *self.state.lock().unwrap() = AppState::Ready;
        }
    }

    /// Called by Kotlin JNI when a binary message arrives from the relay
    pub fn on_cellular_message(&self, data: Vec<u8>) {
        let mut transport = self.transport.lock().unwrap();
        transport.on_cellular_message(data);
    }

    /// Called by Kotlin JNI when the WebSocket has an error
    pub fn on_cellular_error(&self, error: &str) {
        let mut transport = self.transport.lock().unwrap();
        transport.on_cellular_error(error);
    }

    /// Poll outbound cellular queue (called by Kotlin timer)
    pub fn poll_cellular_outbound(&self) -> Option<Vec<u8>> {
        self.transport.lock().unwrap().poll_cellular_outbound()
    }

    /// Get cellular transport state
    pub fn get_cellular_state(&self) -> CellularState {
        self.transport.lock().unwrap().cellular_state()
    }

    /// Get cellular stats JSON
    pub fn get_cellular_stats(&self) -> String {
        self.transport.lock().unwrap().get_cellular_stats()
    }

    // ── Disconnect ──

    pub fn disconnect(&self) -> Result<(), String> {
        info!("StateMachine: disconnecting");
        *self.state.lock().unwrap() = AppState::Disconnecting;

        self.stop_audio_threads();
        self.audio_cache.lock().unwrap().clear();

        let mut transport = self.transport.lock().unwrap();
        transport.disconnect()?;
        *self.state.lock().unwrap() = AppState::Ready;
        Ok(())
    }

    // ── WiFi ──

    pub fn init_wifi(&self) -> Result<(), String> {
        self.transport.lock().unwrap().init_wifi()
    }

    pub fn get_wifi_state(&self) -> crate::wifi_transport::WifiState {
        self.transport.lock().unwrap().wifi_state()
    }

    pub fn get_wifi_peers(&self) -> Vec<crate::wifi_transport::WifiPeer> {
        self.transport.lock().unwrap().get_wifi_peers().to_vec()
    }

    pub fn has_wifi_peers(&self) -> bool {
        self.transport.lock().unwrap().has_wifi_peers()
    }

    // ── WiFi Direct ──

    pub fn get_wifi_direct_state(&self) -> WifiDirectState {
        self.transport.lock().unwrap().wifi_direct_state()
    }

    pub fn get_wifi_direct_peers(&self) -> Vec<WifiDirectPeer> {
        self.transport.lock().unwrap().get_wifi_direct_peers().to_vec()
    }

    pub fn has_wifi_direct_peers(&self) -> bool {
        self.transport.lock().unwrap().has_wifi_direct_peers()
    }

    pub fn get_wifi_direct_role(&self) -> GroupRole {
        self.transport.lock().unwrap().wifi_direct_role()
    }

    // ── Transport ──

    pub fn get_active_transport(&self) -> ActiveTransport {
        self.transport.lock().unwrap().active_transport()
    }

    pub fn is_encrypted(&self) -> bool {
        self.transport.lock().unwrap().is_encrypted()
    }

    pub fn set_crypto_session(&self, session: CryptoSession) {
        self.transport.lock().unwrap().set_crypto(session);
        info!("StateMachine: crypto session set");
    }

    pub fn set_psk(&self, key: &[u8; 32]) {
        self.transport.lock().unwrap().set_psk(key);
    }

    pub fn get_transport(&self) -> &Arc<Mutex<TransportManager>> {
        &self.transport
    }

    pub fn get_device_name(&self) -> String {
        self.device_name.clone()
    }

    /// Set the device display name (called from Kotlin with the actual Android device model)
    pub fn set_device_name(&mut self, name: String) {
        info!("StateMachine: device name set to '{}'", name);
        self.device_name = name.clone();
        self.local_sender_id = crate::users::UserRegistry::derive_user_id(name.as_bytes());
        // Update transport too
        let mut transport = self.transport.lock().unwrap();
        transport.set_device_name(&name);
    }

    // ── PTT ──

    pub fn on_ptt_press(&self) -> Result<(), String> {
        self.ptt_pressed.store(true, Ordering::SeqCst);
        *self.state.lock().unwrap() = AppState::Transmitting;
        info!("PTT pressed");
        Ok(())
    }

    pub fn on_ptt_release(&self) -> Result<(), String> {
        self.ptt_pressed.store(false, Ordering::SeqCst);
        *self.state.lock().unwrap() = AppState::Connected;
        info!("PTT released");
        Ok(())
    }

    pub fn is_ptt_active(&self) -> bool {
        self.ptt_pressed.load(Ordering::SeqCst)
    }

    // ── State / Accessors ──

    pub fn get_state(&self) -> AppState {
        *self.state.lock().unwrap()
    }

    pub fn get_audio_cache(&self) -> &Arc<Mutex<AudioCache>> {
        &self.audio_cache
    }

    pub fn get_user_registry(&self) -> &Arc<Mutex<UserRegistry>> {
        &self.user_registry
    }
}

impl Drop for StateMachine {
    fn drop(&mut self) {
        self.stop_audio_threads();
        // disconnect from transport
        let mut transport = self.transport.lock().unwrap();
        let _ = transport.disconnect();
    }
}
