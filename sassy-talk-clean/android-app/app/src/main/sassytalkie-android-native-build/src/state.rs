use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use log::{error, info, warn};

use crate::audio::AudioEngine;
use crate::bluetooth::{BluetoothDevice, ConnectionState};
use crate::transport::{TransportManager, ActiveTransport};
use crate::codec::{VoiceEncoder, VoiceDecoder, CODEC_FRAME_SIZE};
use crate::audio_cache::AudioCache;
use crate::users::UserRegistry;

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum AppState {
    Initializing, Ready, Connecting, Connected, Transmitting, Receiving, Disconnecting, Error,
}

#[allow(dead_code)]
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
}

impl StateMachine {
    pub fn new(ptt: Arc<AtomicBool>, channel: Arc<AtomicU8>) -> Self {
        let device_name = "SassyTalkie-Android".to_string();
        Self {
            state: Arc::new(Mutex::new(AppState::Initializing)),
            transport: Arc::new(Mutex::new(TransportManager::new(&device_name).unwrap())),
            audio: Arc::new(Mutex::new(AudioEngine::new().unwrap())),
            audio_cache: Arc::new(Mutex::new(AudioCache::new())),
            user_registry: Arc::new(Mutex::new(UserRegistry::new())),
            ptt_pressed: ptt, current_channel: channel,
            tx_running: Arc::new(AtomicBool::new(false)),
            rx_running: Arc::new(AtomicBool::new(false)),
            device_name,
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

    pub fn is_bluetooth_enabled(&self) -> bool {
        self.transport.lock().unwrap().is_bluetooth_enabled()
    }

    pub fn enable_bluetooth(&self) -> Result<(), String> {
        self.transport.lock().unwrap().enable_bluetooth()
    }

    pub fn get_paired_devices(&self) -> Result<Vec<BluetoothDevice>, String> {
        self.transport.lock().unwrap().get_paired_devices()
    }

    pub fn get_connected_device(&self) -> Option<BluetoothDevice> {
        self.transport.lock().unwrap().get_connected_device()
    }

    pub fn connect_to_device(&self, address: &str) -> Result<(), String> {
        *self.state.lock().unwrap() = AppState::Connecting;
        self.transport.lock().unwrap().connect_bluetooth(address)?;
        *self.state.lock().unwrap() = AppState::Connected;
        self.start_rx_thread();
        Ok(())
    }

    pub fn start_listening(&self) -> Result<(), String> {
        *self.state.lock().unwrap() = AppState::Connecting;
        self.transport.lock().unwrap().listen_bluetooth()?;
        // Wait for connection in background
        let state = Arc::clone(&self.state);
        let transport = Arc::clone(&self.transport);
        let rx_running = Arc::clone(&self.rx_running);
        let audio = Arc::clone(&self.audio);
        let audio_cache = Arc::clone(&self.audio_cache);
        let _ptt = Arc::clone(&self.ptt_pressed);
        let channel = Arc::clone(&self.current_channel);

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(500));
                let bt_state = transport.lock().unwrap().bt_state();
                if bt_state == ConnectionState::Connected {
                    *state.lock().unwrap() = AppState::Connected;
                    info!("StateMachine: incoming connection accepted");
                    // Start RX
                    rx_running.store(true, Ordering::Relaxed);
                    Self::rx_loop(Arc::clone(&transport), Arc::clone(&audio), Arc::clone(&audio_cache), Arc::clone(&rx_running), Arc::clone(&channel));
                    break;
                }
                if bt_state == ConnectionState::Disconnected {
                    break;
                }
            }
        });
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        *self.state.lock().unwrap() = AppState::Disconnecting;
        self.tx_running.store(false, Ordering::Relaxed);
        self.rx_running.store(false, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(100));
        self.transport.lock().unwrap().disconnect()?;
        let _ = self.audio.lock().unwrap().release();
        *self.state.lock().unwrap() = AppState::Ready;
        Ok(())
    }

    pub fn on_ptt_press(&self) -> Result<(), String> {
        if *self.state.lock().unwrap() != AppState::Connected {
            return Err("Not connected".into());
        }
        *self.state.lock().unwrap() = AppState::Transmitting;
        self.start_tx_thread();
        Ok(())
    }

    pub fn on_ptt_release(&self) -> Result<(), String> {
        self.tx_running.store(false, Ordering::Relaxed);
        if *self.state.lock().unwrap() == AppState::Transmitting {
            *self.state.lock().unwrap() = AppState::Connected;
        }
        Ok(())
    }

    fn start_tx_thread(&self) {
        if self.tx_running.load(Ordering::Relaxed) { return; }
        self.tx_running.store(true, Ordering::Relaxed);

        let transport = Arc::clone(&self.transport);
        let audio = Arc::clone(&self.audio);
        let tx_running = Arc::clone(&self.tx_running);
        let channel = Arc::clone(&self.current_channel);

        thread::spawn(move || {
            info!("TX thread started");
            let mut encoder = VoiceEncoder::new();
            let mut pcm_buf = vec![0i16; CODEC_FRAME_SIZE];

            if let Err(e) = audio.lock().unwrap().start_recording() {
                error!("Failed to start recording: {}", e);
                tx_running.store(false, Ordering::Relaxed);
                return;
            }

            while tx_running.load(Ordering::Relaxed) {
                let read_result = audio.lock().unwrap().read_audio(&mut pcm_buf);
                match read_result {
                    Ok(n) if n == CODEC_FRAME_SIZE => {
                        let compressed = encoder.encode(&pcm_buf);
                        let ch = channel.load(Ordering::Relaxed);
                        // Prepend channel byte
                        let mut packet = Vec::with_capacity(1 + compressed.len());
                        packet.push(ch);
                        packet.extend_from_slice(&compressed);
                        if let Err(e) = transport.lock().unwrap().send(&packet) {
                            warn!("TX send failed: {}", e);
                        }
                    }
                    Ok(_) => {} // Partial read, skip
                    Err(e) => { warn!("TX read error: {}", e); }
                }
            }

            let _ = audio.lock().unwrap().stop_recording();
            info!("TX thread stopped");
        });
    }

    fn start_rx_thread(&self) {
        if self.rx_running.load(Ordering::Relaxed) { return; }
        self.rx_running.store(true, Ordering::Relaxed);

        let transport = Arc::clone(&self.transport);
        let audio = Arc::clone(&self.audio);
        let audio_cache = Arc::clone(&self.audio_cache);
        let rx_running = Arc::clone(&self.rx_running);
        let channel = Arc::clone(&self.current_channel);

        Self::rx_loop(transport, audio, audio_cache, rx_running, channel);
    }

    fn rx_loop(
        transport: Arc<Mutex<TransportManager>>,
        audio: Arc<Mutex<AudioEngine>>,
        audio_cache: Arc<Mutex<AudioCache>>,
        rx_running: Arc<AtomicBool>,
        channel: Arc<AtomicU8>,
    ) {
        thread::spawn(move || {
            info!("RX thread started");
            let mut decoder = VoiceDecoder::new();
            let mut recv_buf = vec![0u8; 2048];

            if let Err(e) = audio.lock().unwrap().start_playing() {
                error!("Failed to start playback: {}", e);
                rx_running.store(false, Ordering::Relaxed);
                return;
            }

            while rx_running.load(Ordering::Relaxed) {
                let n = match transport.lock().unwrap().receive(&mut recv_buf) {
                    Ok(n) => n,
                    Err(e) => { warn!("RX receive error: {}", e); 0 }
                };

                if n == 0 {
                    // Tick the audio cache for speech gap detection
                    let mut cache = audio_cache.lock().unwrap();
                    cache.tick();
                    // Check for queued playback
                    if let Some((_sender, samples)) = cache.next_playback_frame() {
                        let _ = audio.lock().unwrap().write_audio(&samples);
                    }
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                // Parse: [channel:1][compressed_audio:484]
                if n < 2 { continue; }
                let pkt_channel = recv_buf[0];
                let my_channel = channel.load(Ordering::Relaxed);
                if pkt_channel != my_channel { continue; } // Channel filter

                let compressed = &recv_buf[1..n];
                let pcm = decoder.decode(compressed);

                // Feed into audio cache
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                let mut cache = audio_cache.lock().unwrap();
                if let Some(samples) = cache.ingest_frame("remote", now, pcm.clone()) {
                    // Live mode - play immediately
                    let _ = audio.lock().unwrap().write_audio(&samples);
                }
                cache.tick();
                if let Some((_sender, samples)) = cache.next_playback_frame() {
                    let _ = audio.lock().unwrap().write_audio(&samples);
                }
            }

            let _ = audio.lock().unwrap().stop_playing();
            info!("RX thread stopped");
        });
    }

    pub fn get_state(&self) -> AppState {
        *self.state.lock().unwrap()
    }

    pub fn get_active_transport(&self) -> ActiveTransport {
        self.transport.lock().unwrap().active_transport()
    }

    pub fn get_device_name(&self) -> String {
        self.transport.lock().unwrap().device_name().to_string()
    }

    pub fn init_wifi(&self) -> Result<(), String> {
        self.transport.lock().unwrap().init_wifi()
    }

    pub fn wifi_state(&self) -> crate::wifi_transport::WifiState {
        self.transport.lock().unwrap().wifi_state()
    }

    pub fn get_wifi_peers_json(&self) -> String {
        let transport = self.transport.lock().unwrap();
        let peers = transport.get_wifi_peers();
        let arr: Vec<String> = peers.iter().map(|p| {
            format!("{{\"address\":\"{}\",\"device_name\":\"{}\",\"channel\":{}}}", p.address, p.device_name, p.channel)
        }).collect();
        format!("[{}]", arr.join(","))
    }

    pub fn has_wifi_peers(&self) -> bool {
        self.transport.lock().unwrap().has_wifi_peers()
    }

    pub fn is_encrypted(&self) -> bool {
        self.transport.lock().unwrap().is_encrypted()
    }
}
